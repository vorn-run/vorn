//! A session's git, as the screen shows it: the changes count each card
//! carries (`useGitDiffPolling`), the Changes panel and its commit dialog
//! (`RightPanel`, `CommitDialog`), the branch switcher (`BranchPicker`), the
//! editors it can open in (`OpenInButton`), and removing a worktree
//! (`WorktreeCleanupDialog`, `WorktreeCleanupToastBridge`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::tasks::{Commit, Diff};

/// How often every session's changes and branch are read again (`POLL_INTERVAL`).
pub const POLL: Duration = Duration::from_secs(30);
/// The Changes panel's width when opened, and its limits.
pub const PANEL_W: f32 = 480.0;
pub const PANEL_MIN: f32 = 320.0;
pub const PANEL_MAX: f32 = 800.0;

/// The Changes panel's diff controls, shared with the task panel's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiffOp {
    PickFile(Option<String>),
    /// A click on a changed line: open or close its comment box.
    Line(String, usize),
    AddComment,
    CancelComment,
    RemoveComment(usize),
    ClearComments,
    SendFeedback,
}

/// The commit dialog's controls.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CommitOp {
    Open,
    ToggleUnstaged,
    Push(bool),
    Commit,
    Close,
}

/// What the git controls do.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GitAct {
    /// `GitChangesIndicator`: a session's changes in the panel.
    Open(String),
    Close,
    Maximize,
    Refresh,
    Diff(DiffOp),
    Commit(CommitOp),
    /// The branch switcher on a session's chip.
    Branches(String),
    Switch(String),
    FetchRemotes,
    /// Opens a path in an editor, the default when none is named.
    OpenIn(String, Option<String>),
    /// The editors' menu for a session's project.
    OpenInMenu(String),
    /// A worktree's remove, asking first when sessions or changes would go.
    Remove {
        project: String,
        worktree: String,
    },
    ConfirmRemoval,
    CancelRemoval,
    /// The ended-worktree toast's answers.
    Keep(String),
    RemoveOffered(String),
}

/// `cwd`: where a session's git and files are, its worktree or project.
pub fn session_cwd(info: &vorn_protocol::TerminalSession) -> String {
    info.worktree_path
        .clone()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| info.project_path.clone())
}

/// `GitDiffStat`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stat {
    pub files: u64,
    pub insertions: u64,
    pub deletions: u64,
}

impl Stat {
    pub fn from_value(v: &Value) -> Option<Stat> {
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        v.is_object().then(|| Stat {
            files: n("filesChanged"),
            insertions: n("insertions"),
            deletions: n("deletions"),
        })
    }

    /// Whether there is anything to show or commit.
    pub fn changed(&self) -> bool {
        self.insertions > 0 || self.deletions > 0
    }
}

/// `{n} file(s)`.
pub fn files_label(n: u64) -> String {
    format!("{n} file{}", if n == 1 { "" } else { "s" })
}

/// One editor or app a project can be opened in (`DetectedIDE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ide {
    pub id: String,
    pub name: String,
}

impl Ide {
    pub fn from_value(v: &Value) -> Option<Ide> {
        Some(Ide {
            id: v.get("id")?.as_str()?.to_owned(),
            name: v.get("name")?.as_str()?.to_owned(),
        })
    }
}

/// The glyph today's app draws for an editor (`IDE_ICONS`).
pub fn ide_icon(id: &str) -> Option<&'static str> {
    Some(match id {
        "vscode" | "vscode-insiders" => include_str!("../assets/ide/vscode.svg"),
        "cursor" => include_str!("../assets/ide/cursor.svg"),
        "windsurf" => include_str!("../assets/ide/windsurf.svg"),
        "zed" => include_str!("../assets/ide/zed.svg"),
        "sublime" => include_str!("../assets/ide/sublime.svg"),
        "webstorm" => include_str!("../assets/ide/webstorm.svg"),
        "intellij" => include_str!("../assets/ide/intellij.svg"),
        "xcode" => include_str!("../assets/ide/xcode.svg"),
        "terminal" => include_str!("../assets/ide/terminal.svg"),
        "finder" => include_str!("../assets/ide/finder.svg"),
        _ => return None,
    })
}

/// The branch switcher's list (`BranchPicker`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Branches {
    /// The session it switches.
    pub session: String,
    pub local: Vec<String>,
    pub remote: Vec<String>,
    pub current: Option<String>,
    pub loading: bool,
    pub fetching: bool,
    pub switching: bool,
}

impl Branches {
    /// The branches the filter keeps, remote ones last and marked.
    pub fn shown(&self, filter: &str) -> Vec<(&str, bool)> {
        let lc = filter.to_lowercase();
        self.local
            .iter()
            .map(|b| (b.as_str(), false))
            .chain(self.remote.iter().map(|b| (b.as_str(), true)))
            .filter(|(b, _)| lc.is_empty() || b.to_lowercase().contains(&lc))
            .collect()
    }
}

/// Whether a worktree has uncommitted changes, once asked (`DirtyState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dirty {
    Checking,
    Clean,
    Dirty,
    Unknown,
}

/// The remove-worktree dialog (`ExplicitDeleteInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub project: String,
    pub worktree: String,
    pub sessions: Vec<String>,
    pub dirty: Dirty,
}

impl Removal {
    /// `showWarning`: changes would be lost, or could not be checked.
    pub fn warns(&self) -> bool {
        matches!(self.dirty, Dirty::Dirty | Dirty::Unknown)
    }

    /// The remove button's label.
    pub fn action_label(&self) -> &'static str {
        if self.dirty == Dirty::Checking {
            "Checking\u{2026}"
        } else if !self.sessions.is_empty() {
            "Close sessions & remove"
        } else if self.warns() {
            "Remove anyway"
        } else {
            "Remove"
        }
    }
}

/// The offer to remove a worktree when its last session ended, kept until
/// answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub project: String,
    pub worktree: String,
    /// `None` while the changes check runs; then whether it found any, or
    /// failed (`Err`).
    pub dirty: Option<Result<bool, ()>>,
}

impl Offer {
    /// The toast's sentence, as the bridge words each state.
    pub fn message(&self) -> String {
        let name = crate::state::basename(&self.worktree);
        match self.dirty {
            Some(Ok(true)) => format!("Worktree \"{name}\" has uncommitted changes"),
            Some(Err(())) => {
                format!("Worktree \"{name}\" \u{2014} last session ended (changes check failed)")
            }
            _ => format!("Worktree \"{name}\" \u{2014} last session ended"),
        }
    }

    /// The toast warns, and removing forces.
    pub fn warns(&self) -> bool {
        matches!(self.dirty, Some(Ok(true)) | Some(Err(())))
    }
}

/// A removal in flight: the progress toast's state (`withProgressToast`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removing {
    pub project: String,
    pub worktree: String,
    pub label: &'static str,
}

/// What the screen keeps of git besides vornd's answers.
#[derive(Debug, Default)]
pub struct Git {
    /// Each session's changes, once read.
    pub stats: HashMap<String, Stat>,
    pub polled: Option<Instant>,
    /// Sessions whose poll is on its way.
    pub polling: usize,
    /// The session whose Changes panel is open (`diffSidebarTerminalId`).
    pub panel: Option<String>,
    pub maximized: bool,
    pub width: Option<f32>,
    pub diff: Diff,
    pub commit: Commit,
    pub branches: Option<Branches>,
    /// Detected editors, the default first; `None` until asked.
    pub ides: Option<Vec<Ide>>,
    pub removal: Option<Removal>,
    pub offers: Vec<Offer>,
    pub removing: Vec<Removing>,
    /// Projects whose worktrees have been asked for.
    pub listed: std::collections::HashSet<String>,
}

impl Git {
    pub fn width(&self) -> f32 {
        self.width.unwrap_or(PANEL_W).clamp(PANEL_MIN, PANEL_MAX)
    }

    /// Whether the poll is due.
    pub fn due(&self, now: Instant) -> bool {
        self.polling == 0 && self.polled.is_none_or(|t| now.duration_since(t) >= POLL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stats_read_as_vornd_writes_them() {
        let s = Stat::from_value(&json!({ "filesChanged": 2, "insertions": 4, "deletions": 1 }));
        assert_eq!(
            s,
            Some(Stat {
                files: 2,
                insertions: 4,
                deletions: 1
            })
        );
        assert!(s.unwrap().changed());
        assert_eq!(Stat::from_value(&Value::Null), None);
        assert!(!Stat::default().changed());
        assert_eq!(files_label(1), "1 file");
        assert_eq!(files_label(3), "3 files");
    }

    #[test]
    fn the_branch_filter_keeps_remote_ones_last() {
        let b = Branches {
            local: vec!["main".into(), "feature-x".into()],
            remote: vec!["origin/feature-y".into()],
            ..Branches::default()
        };
        assert_eq!(
            b.shown("FEAT"),
            vec![("feature-x", false), ("origin/feature-y", true)]
        );
        assert_eq!(b.shown("").len(), 3);
        assert!(b.shown("zzz").is_empty());
    }

    #[test]
    fn removal_words_follow_what_is_at_stake() {
        let mut r = Removal {
            project: "/p".into(),
            worktree: "/w".into(),
            sessions: Vec::new(),
            dirty: Dirty::Checking,
        };
        assert_eq!(r.action_label(), "Checking\u{2026}");
        r.dirty = Dirty::Clean;
        assert_eq!(r.action_label(), "Remove");
        r.dirty = Dirty::Unknown;
        assert_eq!(r.action_label(), "Remove anyway");
        r.sessions.push("s".into());
        assert_eq!(r.action_label(), "Close sessions & remove");
    }

    #[test]
    fn the_offer_says_what_the_check_found() {
        let mut o = Offer {
            project: "/p".into(),
            worktree: "/p/.wt/feature".into(),
            dirty: None,
        };
        assert_eq!(
            o.message(),
            "Worktree \"feature\" \u{2014} last session ended"
        );
        assert!(!o.warns());
        o.dirty = Some(Ok(true));
        assert_eq!(o.message(), "Worktree \"feature\" has uncommitted changes");
        assert!(o.warns());
        o.dirty = Some(Err(()));
        assert!(o.message().ends_with("(changes check failed)"));
    }

    #[test]
    fn the_panel_keeps_to_its_limits() {
        let mut g = Git::default();
        assert_eq!(g.width(), PANEL_W);
        g.width = Some(2000.0);
        assert_eq!(g.width(), PANEL_MAX);
        let now = Instant::now();
        assert!(g.due(now));
        g.polled = Some(now);
        assert!(!g.due(now));
    }
}

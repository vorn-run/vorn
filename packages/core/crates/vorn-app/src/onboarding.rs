//! The welcome guide (`OnboardingModal`): seven steps that introduce the
//! app, shown once on a first run and again from the palette or the
//! shortcuts panel. The steps, their copy and the setup commands are
//! `onboarding-data.ts` and `mcp-data.ts` as values.

use web_time::Instant;

use crate::agents::{AgentInfo, Agents};
use serde_json::{json, Value};

/// `ONBOARDING_VERSION`: a `defaults.hasSeenOnboarding` below it opens
/// the guide on launch (2 is the seven-step guide).
pub const VERSION: u64 = 2;

/// What a welcome guide control does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OnbAct {
    /// Closes the guide and remembers it was seen.
    Close,
    /// The next step, or closes the guide from the last.
    Next,
    Back,
    /// A progress bar: straight to that step.
    Go(usize),
    /// Asks vornd again which agents are installed.
    Redetect,
    /// Folds the skills and MCP setup open or shut.
    ToggleMcp,
    /// Copies a setup command, showing a check for a moment.
    Copy(String),
    /// Closes the guide, then does what the button names.
    Then(Then),
}

/// What a button in a step opens once the guide is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Then {
    AddProject,
    OpenTasks,
    NewSession,
    Palette,
    /// Settings at a category (`notifications`, `ssh`, ...).
    Settings(&'static str),
}

/// The guide's state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Onboard {
    pub step: usize,
    /// The skills and MCP setup is open.
    pub mcp_open: bool,
    /// The command last copied, and when, for its check mark.
    pub copied: Option<(String, Instant)>,
    /// What the last detection found, shown while the next one runs.
    pub detected: Option<Vec<String>>,
}

impl Onboard {
    pub fn is_last(&self) -> bool {
        self.step + 1 == STEPS.len()
    }

    /// One step on, stopping at the last.
    pub fn next(&mut self) {
        self.step = (self.step + 1).min(STEPS.len() - 1);
    }

    /// One step back, stopping at the first.
    pub fn back(&mut self) {
        self.step = self.step.saturating_sub(1);
    }

    /// Straight to step `i`, when there is one.
    pub fn go(&mut self, i: usize) {
        if i < STEPS.len() {
            self.step = i;
        }
    }

    pub fn current(&self) -> &'static Step {
        &STEPS[self.step.min(STEPS.len() - 1)]
    }
}

/// One step's header.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    /// The lucide glyph in the header's box.
    pub icon: &'static str,
}

/// `ONBOARDING_STEPS`.
pub const STEPS: [Step; 7] = [
    Step {
        id: "agents",
        title: "Coding Agents",
        subtitle: "Detect agents & install skills",
        icon: "bot",
    },
    Step {
        id: "projects",
        title: "Projects & Tasks",
        subtitle: "Organize repos, branches & work items",
        icon: "folder-git-2",
    },
    Step {
        id: "sessions",
        title: "Launch Sessions",
        subtitle: "Start agents with the right context",
        icon: "rocket",
    },
    Step {
        id: "workspace",
        title: "Your Workspace",
        subtitle: "Grid, tabs & the review panel",
        icon: "layout-dashboard",
    },
    Step {
        id: "workflows",
        title: "Workflows & Schedules",
        subtitle: "Automate multi-agent pipelines",
        icon: "workflow",
    },
    Step {
        id: "shortcuts",
        title: "Command Palette & Shortcuts",
        subtitle: "Find anything instantly",
        icon: "search",
    },
    Step {
        id: "ready",
        title: "You're Ready",
        subtitle: "Start building with your agents",
        icon: "sparkles",
    },
];

/// What the MCP setup says the tools let an agent do.
pub const MCP_CAPS: [&str; 4] = [
    "Launch sessions",
    "Manage tasks",
    "Read terminals",
    "Trigger workflows",
];

/// Whether `agent` counts as installed: before any detection answers,
/// every agent does (`DEFAULT_STATUS`).
pub fn is_installed(installed: Option<&[String]>, agent: &str) -> bool {
    installed.is_none_or(|i| i.iter().any(|a| a == agent))
}

/// The agents whose setups show: those installed, or the first two when
/// none is.
pub fn mcp_to_show<'a>(agents: &'a Agents, installed: Option<&[String]>) -> Vec<&'a AgentInfo> {
    let with_setup = || agents.iter().filter(|a| a.mcp.is_some());
    let found: Vec<_> = with_setup()
        .filter(|a| is_installed(installed, &a.key))
        .collect();
    if found.is_empty() {
        with_setup().take(2).collect()
    } else {
        found
    }
}

/// `EXPLORE_ITEMS`: the last step's links into Settings.
pub const EXPLORE: [(&str, &str, &str); 4] = [
    ("bell", "Enable notifications & sounds", "notifications"),
    ("monitor", "Set up remote hosts (SSH)", "ssh"),
    ("palette", "Customize appearance", "appearance"),
    ("settings", "Configure agent commands", "agents"),
];

/// `ESSENTIAL_SHORTCUTS`: description and keys, with `m` the modifier's
/// label and `a` Alt's.
pub fn essential_shortcuts(m: &str, a: &str) -> [(&'static str, String); 6] {
    [
        ("Command palette", format!("{m}K")),
        ("New session", format!("{m}N")),
        ("View options", format!("{m}J")),
        ("Toggle sidebar", format!("{m}B")),
        ("Keyboard shortcuts", format!("{m}/")),
        ("Show all", format!("{a}1")),
    ]
}

/// Whether this config's user has seen the current guide.
pub fn seen(config: &Value) -> bool {
    let v = config.pointer("/defaults/hasSeenOnboarding");
    // `Number(x ?? 0)`: a number, or a numeric string, counts.
    let n = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0.0),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        _ => 0.0,
    };
    n >= VERSION as f64
}

/// `config` with the guide marked seen, or `None` when it already was.
pub fn mark_seen(config: &Value) -> Option<Value> {
    if seen(config) {
        return None;
    }
    let mut config = config.clone();
    if !config.is_object() {
        config = json!({});
    }
    if !config.get("defaults").is_some_and(Value::is_object) {
        config["defaults"] = json!({});
    }
    config["defaults"]["hasSeenOnboarding"] = json!(VERSION);
    Some(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_stop_at_both_ends() {
        let mut o = Onboard::default();
        o.back();
        assert_eq!(o.step, 0);
        for _ in 0..10 {
            o.next();
        }
        assert_eq!(o.step, 6);
        assert!(o.is_last());
        assert_eq!(o.current().title, "You're Ready");
        o.go(9);
        assert_eq!(o.step, 6);
        o.go(2);
        assert_eq!(o.current().id, "sessions");
    }

    #[test]
    fn a_missing_or_old_version_opens_the_guide() {
        assert!(!seen(&json!({})));
        assert!(!seen(&json!({ "defaults": { "hasSeenOnboarding": 1 } })));
        assert!(!seen(&json!({ "defaults": { "hasSeenOnboarding": true } })));
        assert!(seen(&json!({ "defaults": { "hasSeenOnboarding": 2 } })));
        assert!(seen(&json!({ "defaults": { "hasSeenOnboarding": "3" } })));
    }

    #[test]
    fn marking_seen_keeps_the_rest_of_the_config() {
        let c = json!({ "projects": [1], "defaults": { "fontSize": 13 } });
        let m = mark_seen(&c).expect("not seen yet");
        assert_eq!(m["defaults"]["hasSeenOnboarding"], 2);
        assert_eq!(m["defaults"]["fontSize"], 13);
        assert_eq!(m["projects"], json!([1]));
        assert_eq!(mark_seen(&m), None);
        let bare = mark_seen(&json!({ "defaults": null })).expect("adds defaults");
        assert_eq!(bare["defaults"]["hasSeenOnboarding"], 2);
    }

    #[test]
    fn mcp_setups_follow_what_is_installed() {
        let agents = crate::agents::testing::agents();
        let keys = |l: Vec<&AgentInfo>| l.iter().map(|a| a.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(mcp_to_show(&agents, None)), ["acme", "beta"]);
        let beta = ["beta".to_owned()];
        assert_eq!(keys(mcp_to_show(&agents, Some(&beta))), ["beta"]);
        assert_eq!(keys(mcp_to_show(&agents, Some(&[]))), ["acme", "beta"]);
    }
}

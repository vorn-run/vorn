//! The settings page and its panels: which panel is open,
//! what each panel holds that vornd does not, and the wording the panels
//! derive from vornd's answers (the updater, the server's runtime, the
//! worktree manager's totals and selection).

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// A panel of the settings page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Category {
    #[default]
    Appearance,
    General,
    Notifications,
    /// The agents on this machine and their setup, Vorn's tools included.
    Agents,
    /// The Extensions manager (its route keeps the old `connectors`), where
    /// each connection is an account on its extension's page.
    Connectors,
    Network,
    About,
}

impl Category {
    /// The category an id names.
    pub fn from_id(id: &str) -> Option<Category> {
        Some(match id {
            "appearance" => Category::Appearance,
            "general" => Category::General,
            "notifications" => Category::Notifications,
            "agents" => Category::Agents,
            "connectors" | "extensions" | "plugins" | "connections" => Category::Connectors,
            "network" => Category::Network,
            "about" => Category::About,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::General => "General",
            Category::Notifications => "Notifications",
            Category::Agents => "Agents",
            Category::Connectors => "Extensions",
            Category::Network => "Remote Access",
            Category::About => "About",
        }
    }
}

/// Whether this app runs in a browser, where the machine's own settings
/// (sign-in, updates, the command line tool, the widget) belong to the desktop app.
pub const IN_BROWSER: bool = cfg!(target_arch = "wasm32");

/// Said once a run when vornd found no OS keychain.
pub const NO_KEYCHAIN: &str =
    "No OS keychain found. Vorn keeps secrets in a file only you can read, unencrypted.";

/// How secrets are kept, as a form's hint says it: `keychain` from `vault:status`.
pub fn secrets_kept(keychain: Option<bool>) -> &'static str {
    match keychain {
        Some(false) => "No OS keychain: kept unencrypted in a file only you can read",
        _ => "Encrypted with your OS keychain",
    }
}

/// The sidebar's sections, as `SIDEBAR_SECTIONS` lists them; the
/// EXTENSIONS group follows them, with [`EXTENSIONS`] first.
pub const SECTIONS: [(&str, &[Category]); 2] = [
    (
        "APP SETTINGS",
        &[
            Category::Appearance,
            Category::General,
            Category::Notifications,
        ],
    ),
    ("CONNECTIONS", &[Category::Agents, Category::Network]),
];

/// The EXTENSIONS group's own pages, before each extension's settings.
pub const EXTENSIONS: [Category; 1] = [Category::Connectors];

/// The Connectors built-in's accounts tree, drawn scoped to one connector
/// on that connector's page in Settings › Extensions; never in the nav.
pub const CONNECTIONS_PAGE: &str = "vorn.connectors/settings/connections";

/// The SSH built-in's page, the remote hosts and keys: in the nav, the
/// palette and the guide only while SSH is on, as each settings page is.
pub const SSH_PAGE: &str = "vorn.ssh/settings/hosts";

/// What a settings control does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SetAct {
    Open(Category),
    Close,
    /// `defaults[key] = value`, the value as JSON text.
    Default(String, String),
    /// `defaults.notifications[key] = value`, the value as JSON text.
    Notify(String, String),
    /// The terminal font size, one step down (-1) or up (+1).
    FontSize(i32),
    /// A theme from the picker, by id: its side's key, and the scheme when
    /// a fixed one is of the other side.
    PickTheme(String),
    /// A theme picker chip, on or off.
    ThemeFilter(crate::appearance::ThemeFilter),
    /// Browse, filtered to what adds `tag`, with one listing's page open
    /// when given: Get more themes… and Get more agents….
    Browse(Option<crate::plugins::list::Tag>, Option<String>),
    /// Opens the system's notification settings for Vorn.
    OpenNotifSettings,
    ShellMenu,
    PickShell(String),
    EditShell,
    /// Install, Reinstall, or a notice's Replace and Try again.
    InstallCli,
    RemoveCli,
    /// The notice's administrator prompt that lists this folder in `/etc/paths.d`.
    CliAddToPath(std::path::PathBuf),
    DismissCliNotice,
    CheckUpdates,
    /// Restart into a ready update, download an offered one, or retry.
    UpdateNow,
    /// The sidebar's update notice put away; the gear keeps a dot.
    DismissUpdate,
    // Agents.
    Redetect,
    /// An agent's detail shown in place of the list, or the list again.
    OpenAgent(Option<String>),
    ResetAgent(String),
    /// The detail's Advanced rows, shown or hidden.
    ExpandAgent(String),
    /// Runs the agent's installer in a terminal card.
    InstallAgent(String),
    /// Runs the agent's Vorn tools setup for it.
    ConnectAgent(String),
    /// Removes the agent's adapter; the first press asks in the row.
    RemoveAgent(String),
    /// The Remove row's Cancel.
    KeepAgent,
    /// Typing in an agent's command, arguments or headless arguments.
    AgentText(String, AgentField),
    // About.
    /// End the older holder's terminals; the first press asks in the row.
    EndHolder(String),
    /// The terminals row's Cancel: End them stops asking.
    KeepHolder,
    /// Stop vornd and reopen the app, so the terminals move to this version.
    RestartVorn,
    /// Install each pack that has a newer release.
    UpdateExtensions,
}

/// Which of an agent's command fields a text act edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentField {
    Command,
    Args,
    Headless,
}

/// One of `shell:listInstalled`'s shells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    pub family: String,
    pub name: String,
    pub path: String,
    pub version: Option<String>,
    pub level: String,
    pub limitation: Option<String>,
}

impl Shell {
    pub fn from_value(v: &Value) -> Option<Shell> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        let blocks = v.get("blocks");
        let b = |k: &str| blocks.and_then(|b| b.get(k)).and_then(Value::as_str);
        Some(Shell {
            family: s("family")?,
            name: s("name").unwrap_or_default(),
            path: s("path")?,
            version: s("version"),
            level: b("level").unwrap_or("full").to_owned(),
            limitation: b("limitation").map(str::to_owned),
        })
    }
}

/// `SIGILS`: a shell family's prompt sigil, or a dot for none.
pub fn sigil(family: Option<&str>) -> &'static str {
    match family {
        Some("zsh") => "%",
        Some("bash") => "$",
        Some("fish") => "><>",
        Some("powershell") => "PS",
        Some("cmd") => "C:\\",
        _ => "\u{b7}",
    }
}

/// The updater's state; `Unsupported` in a build tree.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Idle {
        last_checked: Option<f64>,
    },
    Checking,
    Available {
        version: String,
    },
    Downloading {
        version: String,
        percent: u32,
    },
    Ready {
        version: String,
    },
    /// A newer version this install cannot take itself (a .deb).
    Manual {
        version: String,
    },
    Error {
        message: String,
    },
    Unsupported,
}

/// The tones a status can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Blocked,
    Broken,
    Live,
    Settled,
    Idle,
}

/// The action an update state is worth offering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    Restart,
    Download,
    Retry,
}

/// `UpdateStatusView`.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateView {
    pub label: String,
    pub detail: Option<String>,
    pub tone: Tone,
    pub percent: Option<u32>,
    pub action: Option<UpdateAction>,
}

/// When updates were last checked, in words.
pub fn last_checked(at: Option<f64>, now: f64) -> Option<String> {
    let at = at?;
    let seconds = ((now - at) / 1000.0).floor().max(0.0) as u64;
    let plural =
        |n: u64, unit: &str| format!("checked {n} {unit}{} ago", if n == 1 { "" } else { "s" });
    Some(if seconds < 60 {
        "checked just now".to_owned()
    } else if seconds < 3600 {
        plural(seconds / 60, "minute")
    } else if seconds < 86_400 {
        plural(seconds / 3600, "hour")
    } else {
        plural(seconds / 86_400, "day")
    })
}

/// The label, detail, tone and action for the updater's state.
pub fn describe_update(status: &UpdateStatus, channel: &str, now: f64) -> UpdateView {
    let view = |label: String, detail: Option<String>, tone, action| UpdateView {
        label,
        detail,
        tone,
        percent: None,
        action,
    };
    match status {
        UpdateStatus::Idle { last_checked: at } => view(
            "Up to date".into(),
            Some(last_checked(*at, now).unwrap_or_else(|| format!("{channel} channel"))),
            Tone::Idle,
            None,
        ),
        UpdateStatus::Checking => view(
            "Checking for updates\u{2026}".into(),
            Some(format!("{channel} channel")),
            Tone::Live,
            None,
        ),
        UpdateStatus::Available { version } => view(
            format!("Version {version} is available"),
            Some("not downloaded yet".into()),
            Tone::Idle,
            Some(UpdateAction::Download),
        ),
        UpdateStatus::Downloading { version, percent } => UpdateView {
            label: if version.is_empty() {
                "Downloading update".into()
            } else {
                format!("Downloading {version}")
            },
            detail: None,
            tone: Tone::Live,
            percent: Some(*percent),
            action: None,
        },
        UpdateStatus::Ready { version } => view(
            format!("Version {version} is ready to install"),
            Some("Vorn closes for a few seconds and reopens itself".into()),
            Tone::Blocked,
            Some(UpdateAction::Restart),
        ),
        UpdateStatus::Manual { version } => view(
            format!("Version {version} is available"),
            Some("Install the new package from the releases page".into()),
            Tone::Idle,
            None,
        ),
        UpdateStatus::Error { message } => view(
            "Couldn't update".into(),
            Some(message.clone()),
            Tone::Broken,
            Some(UpdateAction::Retry),
        ),
        UpdateStatus::Unsupported => view(
            "Updates are off in development".into(),
            Some("packaged builds only".into()),
            Tone::Idle,
            None,
        ),
    }
}

/// What restarting for an update does to the sessions.
pub fn update_cost(sessions: usize, turn_running: bool, survive: bool) -> Option<String> {
    if sessions == 0 {
        return None;
    }
    let one = sessions == 1;
    let subject = if one {
        "Your session".to_owned()
    } else {
        format!("Your {sessions} sessions")
    };
    let s = if one { "s" } else { "" };
    let fate = if survive {
        format!("keep{s} running through the update")
    } else {
        format!("end{s} with the update")
    };
    let turn = match (turn_running, survive) {
        (false, _) => "",
        (true, true) => " The turn in flight continues.",
        (true, false) => " The turn in flight is cut short.",
    };
    Some(format!("{subject} {fate}.{turn}"))
}

/// A session holder vornd reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: i64,
    pub instance: String,
    pub build: String,
    pub sessions: Option<i64>,
    pub compatible: bool,
}

impl Holder {
    fn from_value(v: &Value) -> Option<Holder> {
        Some(Holder {
            pid: v.get("pid")?.as_i64()?,
            instance: v.get("instance")?.as_str()?.to_owned(),
            build: v
                .get("build")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            sessions: v.get("sessions").and_then(Value::as_i64),
            compatible: v.get("compatible").and_then(Value::as_bool) == Some(true),
        })
    }
}

/// `/vornd/health`'s `sessiond`: the current holder, older ones, an error.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Holders {
    pub current: bool,
    pub older: Vec<Holder>,
    pub error: Option<String>,
}

impl Holders {
    pub fn from_health(v: &Value) -> Option<Holders> {
        let s = v.get("sessiond")?;
        if !s.is_object() {
            return None;
        }
        Some(Holders {
            current: s.get("current").and_then(Holder::from_value).is_some(),
            older: s
                .get("older")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Holder::from_value)
                .collect(),
            error: s.get("error").and_then(Value::as_str).map(str::to_owned),
        })
    }
}

/// The links under About's card.
/// The terminal font sizes a person can pick, in px.
pub const FONT_SIZES: std::ops::RangeInclusive<i64> = 8..=24;

/// The terminal font size one step of `delta` from `cur`, kept in [`FONT_SIZES`].
pub fn step_font_size(cur: i64, delta: i32) -> i64 {
    (cur + i64::from(delta)).clamp(*FONT_SIZES.start(), *FONT_SIZES.end())
}

pub const WEBSITE: &str = "https://vorn.run";
pub const DOCUMENTATION: &str = "https://github.com/vorn-run/vorn/tree/main/docs";
pub const FEEDBACK: &str = "https://github.com/vorn-run/vorn/issues/new";
pub const GITHUB: &str = "https://github.com/vorn-run/vorn";

/// The release notes of `version`, on its release page.
pub fn release_notes(version: &str) -> String {
    format!("{}/tag/v{version}", crate::update::RELEASES)
}

/// About's header row: the version or the update that is ready, a detail
/// line, and the one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutHead {
    pub title: String,
    pub detail: String,
    /// Up to date: the detail leads with a check.
    pub current: bool,
    pub action: Option<UpdateAction>,
}

/// What About's header row says for the updater's `status`, with `sessions`
/// live and this app at `app`.
pub fn about_head(
    status: &UpdateStatus,
    app: &str,
    now: f64,
    sessions: usize,
    survive: bool,
) -> AboutHead {
    let head = |detail: String, current, action| AboutHead {
        title: format!("Vorn {app}"),
        detail,
        current,
        action,
    };
    match status {
        UpdateStatus::Idle { last_checked: at } => head(
            match last_checked(*at, now) {
                Some(when) => format!("Up to date \u{b7} {when}"),
                None => "Up to date".to_owned(),
            },
            true,
            None,
        ),
        UpdateStatus::Checking => head("Checking for updates\u{2026}".to_owned(), false, None),
        UpdateStatus::Available { version } => head(format!("Downloading {version}"), false, None),
        UpdateStatus::Downloading { version, percent } => head(
            format!("Downloading {version} \u{b7} {percent}%"),
            false,
            None,
        ),
        UpdateStatus::Ready { version } => {
            let keep = match (sessions, survive) {
                (0, _) => String::new(),
                (1, true) => " \u{b7} your session keeps running".to_owned(),
                (n, true) => format!(" \u{b7} your {n} sessions keep running"),
                (1, false) => " \u{b7} your session ends".to_owned(),
                (n, false) => format!(" \u{b7} your {n} sessions end"),
            };
            AboutHead {
                title: format!("{version} is ready"),
                detail: format!("You have {app}{keep}"),
                current: false,
                action: Some(UpdateAction::Restart),
            }
        }
        UpdateStatus::Manual { version } => AboutHead {
            title: format!("{version} is available"),
            detail: "Install the new package from the releases page".to_owned(),
            current: false,
            action: None,
        },
        UpdateStatus::Error { message } => head(
            format!("Could not check for updates: {message}"),
            false,
            Some(UpdateAction::Retry),
        ),
        UpdateStatus::Unsupported => head(
            "Updates are off in development builds".to_owned(),
            false,
            None,
        ),
    }
}

/// Terminals that still run on an older build, as About's row says it: the
/// older session holder's (which can be ended), else vornd's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Behind {
    pub title: String,
    pub detail: String,
    /// The older holder's instance, for End them.
    pub holder: Option<String>,
}

/// Whether About shows the terminals row, for vornd at `server`, this app at
/// `app`, and the session holders vornd reports.
pub fn behind(server: Option<&str>, app: &str, holders: Option<&Holders>) -> Option<Behind> {
    const MOVE: &str = "They keep running and move over when Vorn restarts";
    if let Some(h) = holders {
        if let Some(old) = h.older.iter().find(|o| o.sessions != Some(0)) {
            let build = if old.build.is_empty() {
                "an older version"
            } else {
                old.build.as_str()
            };
            return Some(Behind {
                title: format!("Your terminals still run on {build}"),
                detail: if old.compatible {
                    MOVE.to_owned()
                } else {
                    "This version cannot reach them; they keep running until you end them"
                        .to_owned()
                },
                holder: Some(old.instance.clone()),
            });
        }
        if let (Some(e), false) = (&h.error, h.current) {
            return Some(Behind {
                title: "The session holder is not running".to_owned(),
                detail: e.clone(),
                holder: None,
            });
        }
    }
    match server {
        Some(s) if s != app => Some(Behind {
            title: if s == "unknown" {
                "Your terminals run on a server started outside Vorn".to_owned()
            } else {
                format!("Your terminals still run on {s}")
            },
            detail: MOVE.to_owned(),
            holder: None,
        }),
        _ => None,
    }
}

/// What About's Extensions row says: each installed pack up to date, or the
/// ones with a newer release.
pub fn extensions_note(installed: usize, outdated: &[&str]) -> String {
    match (outdated, installed) {
        ([], 0) => "None installed".to_owned(),
        ([], 1) => "Up to date".to_owned(),
        ([], n) => format!("All {n} up to date"),
        (names, _) if names.len() <= 3 => names.join(", "),
        (names, _) => format!("{} and {} more", names[..2].join(", "), names.len() - 2),
    }
}

/// The `vorn` command's state on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliState {
    pub available: bool,
    pub installed: bool,
    /// Where it is, or would go.
    pub path: String,
    pub on_path: bool,
    /// What is in the way, or why it cannot be installed.
    pub note: Option<String>,
    /// In a folder this user cannot change, such as a package's `/usr/bin`.
    pub managed: bool,
}

/// Everything the settings page holds between frames.
#[derive(Debug, Default)]
pub struct Settings {
    pub open: bool,
    pub category: Category,
    /// An extension's settings page shown in place of the category's, by point key.
    pub ext_page: Option<String>,
    pub shells: Option<Vec<Shell>>,
    /// The shell picker's typed path, while it is shown.
    pub shell_editing: bool,
    pub cli: Option<CliState>,
    pub cli_busy: bool,
    pub cli_error: Option<String>,
    pub update: Option<UpdateStatus>,
    /// The sidebar's ready-update notice was dismissed (`updateBannerDismissed`).
    pub update_dismissed: bool,
    pub detecting: bool,
    /// The agents whose Advanced rows are shown.
    pub agents_open: HashSet<String>,
    /// The agent whose detail is open on the Agents page.
    pub agent: Option<String>,
    /// The agent whose Remove asks first.
    pub confirm_remove: Option<String>,
    /// Typed arguments not yet committed, by (agent, field).
    pub args: HashMap<(String, AgentField), String>,
    pub holders: Option<Holders>,
    pub end_failure: Option<String>,
    /// The holder whose End them asks first.
    pub confirm_end: Option<String>,
    /// The Card Row Height slider (`rowHeight`), pixels.
    pub row_height: f32,
    /// The themes extensions bring, and the palette they last resolved to.
    pub appearance: crate::appearance::Appearance,
    /// The theme tile under the pointer, drawn until the pointer leaves it.
    pub theme_preview: Option<String>,
    /// The theme picker's chip on, if any.
    pub theme_filter: Option<crate::appearance::ThemeFilter>,
    /// The installed monospace families, read when the font list first opens.
    pub fonts: Option<std::rc::Rc<[crate::ui::MonoFamily]>>,
}

impl Settings {
    pub fn new() -> Settings {
        Settings {
            row_height: 208.0,
            ..Settings::default()
        }
    }
}

/// An agent's definition (`AGENT_DEFINITIONS`), as its adapter gives it.
pub use crate::agents::{AgentInfo as Agent, McpSetup};

/// `path` with a macOS or Linux home directory written as `~`.
pub fn home_relative(path: &str) -> String {
    for root in ["/Users/", "/home/"] {
        if let Some(rest) = path.strip_prefix(root) {
            if let Some(at) = rest.find('/') {
                return format!("~{}", &rest[at..]);
            }
        }
    }
    path.to_owned()
}

/// `agent:locate`'s answer: the agents found, and where each one's program is.
pub fn located(answer: &Value) -> Option<(Vec<String>, HashMap<String, String>)> {
    let agents = answer.get("agents")?.as_object()?;
    let paths: HashMap<String, String> = agents
        .iter()
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
        .collect();
    let mut found: Vec<String> = paths.keys().cloned().collect();
    found.sort();
    Some((found, paths))
}

/// The default `agentCommands[key]`.
pub fn default_command(a: &Agent) -> Value {
    let mut v = serde_json::json!({ "command": a.command, "args": a.args });
    if !a.headless.is_empty() {
        v["headlessArgs"] = serde_json::json!(a.headless);
    }
    v
}

/// The configured command, else the default.
pub fn command_of(config: &Value, a: &Agent) -> Value {
    config
        .get("agentCommands")
        .and_then(|c| c.get(&a.key))
        .filter(|c| c.is_object())
        .cloned()
        .unwrap_or_else(|| default_command(a))
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// An agent command's `args` or `headlessArgs`, joined with spaces.
pub fn args_text(cmd: &Value, headless: bool) -> String {
    strings(cmd.get(if headless { "headlessArgs" } else { "args" })).join(" ")
}

/// The command differs from its default.
pub fn modified(cmd: &Value, a: &Agent) -> bool {
    let d = default_command(a);
    cmd.get("command") != d.get("command")
        || strings(cmd.get("args")) != strings(d.get("args"))
        || strings(cmd.get("headlessArgs")) != strings(d.get("headlessArgs"))
}

/// Typed arguments as the list they are saved as.
pub fn split_args(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

/// Whether the agent's command is on this machine; until detection first
/// answers, every agent counts, so the list does not flash "Needs setup".
pub fn is_ready(installed: Option<&[String]>, key: &str) -> bool {
    installed.is_none_or(|l| l.iter().any(|k| k == key))
}

/// The agents whose command was found, then the ones that need setup, each
/// in the order Vorn lists them.
pub fn partition_agents<'a>(
    agents: impl IntoIterator<Item = &'a Agent>,
    installed: Option<&[String]>,
) -> (Vec<&'a Agent>, Vec<&'a Agent>) {
    agents
        .into_iter()
        .partition(|a| is_ready(installed, &a.key))
}

/// `defaults[AGENT_TOOLS][agent]`: Vorn ran Connect for the agent and it worked.
pub const AGENT_TOOLS: &str = "agentTools";

/// Whether the agent can use Vorn's tools, as far as Vorn knows.
pub fn tools_connected(config: &Value, agent: &str) -> bool {
    config
        .get("defaults")
        .and_then(|d| d.get(AGENT_TOOLS))
        .and_then(|t| t.get(agent))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// What a setup typed into a shell runs: the commands in order, then `exit`,
/// so the shell's exit code says whether every one of them worked.
pub fn setup_line(commands: &[String]) -> String {
    format!("{} && exit\r", commands.join(" && "))
}

/// How long an agent's session stays quiet before Vorn types into it.
pub const QUIET: std::time::Duration = std::time::Duration::from_millis(1500);

/// The longest Vorn waits for that quiet before typing anyway.
pub const QUIET_CAP: std::time::Duration = std::time::Duration::from_secs(20);

/// A terminal Vorn started to set an agent up, by its session.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentJob {
    /// The installer; detection runs again when its shell exits.
    Install(String),
    /// Setup commands in a shell; connected when the shell exits cleanly.
    Connect(String),
    /// Setup commands typed into a session of the agent, one at a time, each
    /// once the session has gone quiet after `since`.
    ConnectInAgent {
        agent: String,
        pending: std::collections::VecDeque<String>,
        since: web_time::Instant,
    },
}

/// When the next setup command can be typed: once output that came after
/// `since` has been quiet for [`QUIET`], and by [`QUIET_CAP`] after `since`
/// even if the session never printed.
pub fn type_due(
    last_output: Option<web_time::Instant>,
    since: web_time::Instant,
) -> web_time::Instant {
    let cap = since + QUIET_CAP;
    match last_output {
        Some(o) if o >= since => (o + QUIET).min(cap),
        _ => cap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_size_steps_by_one_within_its_range() {
        assert_eq!(step_font_size(13, 1), 14);
        assert_eq!(step_font_size(13, -1), 12);
        assert_eq!(step_font_size(24, 1), 24);
        assert_eq!(step_font_size(8, -1), 8);
        assert_eq!(step_font_size(40, -1), 24);
    }

    #[test]
    fn reads_where_each_agent_was_found() {
        let answer = serde_json::json!({
            "agents": { "acme": "/h/.local/bin/acme", "beta": null, "gamma": "/opt/homebrew/bin/gamma" },
            "shellResolved": true,
        });
        let (found, paths) = located(&answer).expect("answer");
        assert_eq!(found, ["acme", "gamma"]);
        assert_eq!(paths["acme"], "/h/.local/bin/acme");
        assert!(!paths.contains_key("beta"));
        assert_eq!(located(&serde_json::json!({ "acme": true })), None);
        assert_eq!(
            home_relative("/Users/me/.local/bin/acme"),
            "~/.local/bin/acme"
        );
        assert_eq!(
            home_relative("/opt/homebrew/bin/beta"),
            "/opt/homebrew/bin/beta"
        );
    }

    use serde_json::json;

    #[test]
    fn update_states_read_as_the_panel_writes_them() {
        let now = 10_000_000.0;
        let idle = describe_update(&UpdateStatus::Idle { last_checked: None }, "beta", now);
        assert_eq!(idle.label, "Up to date");
        assert_eq!(idle.detail.as_deref(), Some("beta channel"));
        let ago = describe_update(
            &UpdateStatus::Idle {
                last_checked: Some(now - 91_000.0),
            },
            "stable",
            now,
        );
        assert_eq!(ago.detail.as_deref(), Some("checked 1 minute ago"));
        let ready = describe_update(
            &UpdateStatus::Ready {
                version: "0.9.0".into(),
            },
            "stable",
            now,
        );
        assert_eq!(ready.tone, Tone::Blocked);
        assert_eq!(ready.action, Some(UpdateAction::Restart));
        let off = describe_update(&UpdateStatus::Unsupported, "stable", now);
        assert_eq!(off.label, "Updates are off in development");
        assert_eq!(
            last_checked(Some(0.0), 3.0 * 86_400_000.0).as_deref(),
            Some("checked 3 days ago")
        );
    }

    #[test]
    fn the_restart_cost_counts_sessions_and_the_turn() {
        assert_eq!(update_cost(0, true, true), None);
        assert_eq!(
            update_cost(1, false, true).as_deref(),
            Some("Your session keeps running through the update.")
        );
        assert_eq!(
            update_cost(3, true, false).as_deref(),
            Some("Your 3 sessions end with the update. The turn in flight is cut short.")
        );
    }

    #[test]
    fn agent_commands_compare_with_their_defaults() {
        let agents = crate::agents::testing::agents();
        let acme = agents.get("acme").expect("acme");
        let config = json!({});
        let cmd = command_of(&config, acme);
        assert!(!modified(&cmd, acme));
        assert_eq!(args_text(&cmd, true), "--yes");
        let changed =
            json!({ "agentCommands": { "acme": { "command": "acme", "args": ["--x"] } } });
        assert!(modified(&command_of(&changed, acme), acme));
        assert_eq!(split_args("  a   b "), vec!["a", "b"]);
    }

    #[test]
    fn holders_say_what_they_hold() {
        let h = Holders::from_health(&json!({ "sessiond": { "current": { "pid": 1, "instance": "a" },
            "older": [{ "pid": 2, "instance": "b", "build": "0.7", "sessions": 1, "compatible": true }] } }))
        .expect("holders");
        assert!(h.current);
        assert_eq!(h.older[0].build, "0.7");
    }

    #[test]
    fn about_says_the_version_or_the_update_that_is_ready() {
        let idle = about_head(
            &UpdateStatus::Idle { last_checked: None },
            "0.8.0",
            0.0,
            2,
            true,
        );
        assert_eq!(idle.title, "Vorn 0.8.0");
        assert_eq!(idle.detail, "Up to date");
        assert!(idle.current);
        assert_eq!(idle.action, None);
        let ready = UpdateStatus::Ready {
            version: "0.8.1".into(),
        };
        let r = about_head(&ready, "0.8.0", 0.0, 4, true);
        assert_eq!(r.title, "0.8.1 is ready");
        assert_eq!(
            r.detail,
            "You have 0.8.0 \u{b7} your 4 sessions keep running"
        );
        assert_eq!(r.action, Some(UpdateAction::Restart));
        assert_eq!(
            about_head(&ready, "0.8.0", 0.0, 1, false).detail,
            "You have 0.8.0 \u{b7} your session ends"
        );
        let failed = UpdateStatus::Error {
            message: "offline".into(),
        };
        assert_eq!(
            about_head(&failed, "0.8.0", 0.0, 0, true).action,
            Some(UpdateAction::Retry)
        );
        assert_eq!(
            release_notes("0.8.1"),
            "https://github.com/vorn-run/vorn/releases/tag/v0.8.1"
        );
    }

    #[test]
    fn the_terminals_row_shows_only_while_terminals_are_behind() {
        assert_eq!(behind(Some("0.8"), "0.8", None), None);
        let old = |sessions| Holders {
            current: true,
            older: vec![Holder {
                pid: 2,
                instance: "b".into(),
                build: "0.7".into(),
                sessions,
                compatible: true,
            }],
            error: None,
        };
        let b = behind(Some("0.8"), "0.8", Some(&old(Some(3)))).expect("behind");
        assert_eq!(b.title, "Your terminals still run on 0.7");
        assert_eq!(b.holder.as_deref(), Some("b"));
        assert_eq!(behind(Some("0.8"), "0.8", Some(&old(Some(0)))), None);
        let server = behind(Some("0.7"), "0.8", None).expect("vornd behind");
        assert_eq!(server.title, "Your terminals still run on 0.7");
        assert_eq!(server.holder, None);
    }

    #[test]
    fn the_extensions_row_names_what_has_an_update() {
        assert_eq!(extensions_note(12, &[]), "All 12 up to date");
        assert_eq!(extensions_note(0, &[]), "None installed");
        assert_eq!(extensions_note(5, &["Tasks", "Git"]), "Tasks, Git");
        assert_eq!(
            extensions_note(9, &["A", "B", "C", "D", "E"]),
            "A, B and 3 more"
        );
    }

    #[test]
    fn ready_agents_come_first_and_the_rest_need_setup() {
        let agents = crate::agents::testing::agents();
        let keys = |l: Vec<&Agent>| l.iter().map(|a| a.key.clone()).collect::<Vec<_>>();
        let found = vec!["gamma".to_owned(), "acme".to_owned()];
        let (ready, setup) = partition_agents(&agents, Some(&found));
        assert_eq!(keys(ready), ["acme", "gamma"]);
        assert_eq!(keys(setup), ["beta"]);
        let (ready, setup) = partition_agents(&agents, None);
        assert_eq!(ready.len(), 3, "every agent counts until detection answers");
        assert!(setup.is_empty());
        assert!(partition_agents(&agents, Some(&[])).0.is_empty());
    }

    #[test]
    fn vorn_tools_are_connected_only_where_connect_worked() {
        let config = json!({ "defaults": { AGENT_TOOLS: { "acme": true, "beta": false } } });
        assert!(tools_connected(&config, "acme"));
        assert!(!tools_connected(&config, "beta"));
        assert!(!tools_connected(&config, "gamma"));
        assert!(!tools_connected(&json!({}), "acme"));
    }

    #[test]
    fn a_shell_setup_exits_only_when_every_command_worked() {
        let cmds = ["a add vorn".to_owned(), "a install vorn".to_owned()];
        assert_eq!(setup_line(&cmds), "a add vorn && a install vorn && exit\r");
        assert_eq!(setup_line(&cmds[..1]), "a add vorn && exit\r");
    }

    #[test]
    fn types_into_an_agent_once_it_has_gone_quiet() {
        use std::time::Duration;
        let since = web_time::Instant::now();
        let ms = Duration::from_millis;
        // Nothing printed yet: wait up to the cap.
        assert_eq!(type_due(None, since), since + QUIET_CAP);
        // Output from before the last command typed does not count.
        let before = since.checked_sub(ms(10)).unwrap_or(since);
        if before < since {
            assert_eq!(type_due(Some(before), since), since + QUIET_CAP);
        }
        // Output after it: due once quiet.
        assert_eq!(
            type_due(Some(since + ms(400)), since),
            since + ms(400) + QUIET
        );
        // A session that keeps printing still gets typed into at the cap.
        assert_eq!(type_due(Some(since + ms(19_900)), since), since + QUIET_CAP);
    }
}

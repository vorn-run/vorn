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
    Updates,
    Notifications,
    Agents,
    Mcp,
    /// The Extensions manager (its route keeps the old `connectors`).
    Connectors,
    /// Settings › Connections: opens the Connectors built-in's own page,
    /// [`CONNECTIONS_PAGE`].
    Connections,
    Keys,
    Network,
    About,
}

impl Category {
    /// The category an id names.
    pub fn from_id(id: &str) -> Option<Category> {
        Some(match id {
            "appearance" => Category::Appearance,
            "general" => Category::General,
            "updates" => Category::Updates,
            "notifications" => Category::Notifications,
            "agents" => Category::Agents,
            "mcp" => Category::Mcp,
            "connectors" | "extensions" | "plugins" => Category::Connectors,
            "connections" => Category::Connections,
            "keys" => Category::Keys,
            "network" => Category::Network,
            "about" => Category::About,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::General => "General",
            Category::Updates => "Updates",
            Category::Notifications => "Notifications",
            Category::Agents => "Coding Agents",
            Category::Mcp => "Skills & MCP",
            Category::Connectors => "Extensions",
            Category::Connections => "Connections",
            Category::Keys => "Keys",
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

impl Category {
    /// Whether the category has anything to set from here.
    pub fn shown(self) -> bool {
        !(IN_BROWSER && self == Category::Updates)
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
            Category::Updates,
            Category::Notifications,
        ],
    ),
    (
        "CONNECTIONS",
        &[
            Category::Agents,
            Category::Mcp,
            Category::Keys,
            Category::Network,
        ],
    ),
];

/// The EXTENSIONS group's own pages, before each extension's settings.
pub const EXTENSIONS: [Category; 1] = [Category::Connectors];

/// The Connectors built-in's settings page, which every "Settings ›
/// Connections" opens; first of the extensions' pages in the nav.
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
    /// `defaults.notifications[key] = value`.
    Notify(String, bool),
    /// The terminal font size, one step down or up.
    FontSize(i32),
    TestSound,
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
    // Coding agents.
    Redetect,
    ResetAgent(String),
    ExpandAgent(String),
    CopyInstall(String),
    /// Typing in an agent's command, arguments or headless arguments.
    AgentText(String, AgentField),
    // Skills and MCP.
    Copy(String),
    // Keys.
    TestKey(String),
    Rotate(String, String),
    SaveRotate(String, String),
    CancelRotate(String),
    // About.
    EndHolder(String),
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

/// What the Terminal server row says for a server whose build is known.
pub fn describe_server(server: &str, app: &str, adopted: bool) -> (String, &'static str) {
    if server == app {
        let how = if adopted {
            "running since before this window opened"
        } else {
            "started by this window"
        };
        return (format!("Vorn {server}, {how}"), "Current");
    }
    let behind = if server == "unknown" {
        "A server started outside Vorn is holding your terminals".to_owned()
    } else {
        format!("Vorn {server} is still holding your terminals; this app is {app}")
    };
    (
        format!("{behind}. Moving it needs a restart of Vorn on this machine."),
        "Restart to move",
    )
}

/// One of `connection:listKeys`'s entries.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnKey {
    pub connection: String,
    pub name: String,
    pub connector: String,
    pub fields: Vec<KeyFieldInfo>,
    pub usage: u64,
}

/// One field of a [`ConnKey`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFieldInfo {
    pub key: String,
    pub label: String,
    pub readable: bool,
    pub hint: Option<String>,
    pub env: Option<Vec<String>>,
}

impl ConnKey {
    pub fn from_value(v: &Value) -> Option<ConnKey> {
        let s = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).map(str::to_owned);
        Some(ConnKey {
            connection: s(v, "connectionId")?,
            name: s(v, "name").unwrap_or_default(),
            connector: s(v, "connectorId").unwrap_or_default(),
            fields: v
                .get("fields")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|f| {
                    Some(KeyFieldInfo {
                        key: s(f, "key")?,
                        label: s(f, "label").unwrap_or_default(),
                        readable: f.get("readable").and_then(Value::as_bool).unwrap_or(true),
                        hint: s(f, "hint"),
                        env: f.get("envNames").and_then(Value::as_array).map(|a| {
                            a.iter()
                                .filter_map(|e| e.as_str().map(str::to_owned))
                                .collect()
                        }),
                    })
                })
                .collect(),
            usage: v.get("usageCount").and_then(Value::as_u64).unwrap_or(0),
        })
    }
}

/// What a key's stored value can be said to be.
pub fn describe_field(f: &KeyFieldInfo) -> String {
    if !f.readable {
        return "Locked \u{2014} this machine cannot read it".to_owned();
    }
    if let Some(env) = &f.env {
        return if env.is_empty() {
            "No variables set".to_owned()
        } else {
            env.join(" \u{b7} ")
        };
    }
    f.hint
        .clone()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Set".to_owned())
}

/// How many steps use something, in words.
pub fn describe_usage(n: u64) -> String {
    match n {
        0 => "unused".to_owned(),
        1 => "used by 1 step".to_owned(),
        n => format!("used by {n} steps"),
    }
}

/// A key's test result and its rotation in progress.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct KeyRowState {
    pub testing: bool,
    /// The preflight's answer: none, ok, or failed, and its message.
    pub result: Option<(Option<bool>, Option<String>)>,
    pub rotating: Option<String>,
    pub saving: bool,
    pub error: Option<String>,
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

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What an older session holder means for its sessions.
pub fn older_note(h: &Holder) -> String {
    let held = match h.sessions {
        None => "Sessions started before Vorn was updated".to_owned(),
        Some(n) => format!(
            "{} started before Vorn was updated",
            plural(n, "session", "sessions")
        ),
    };
    let verb = if h.sessions == Some(1) { "is" } else { "are" };
    if h.compatible {
        format!(
            "{held} {verb} still on the older session holder ({}). It exits after the last one ends.",
            h.build
        )
    } else {
        format!(
            "{held} {verb} on a session holder ({}) this version cannot talk to. They keep running until you end them.",
            h.build
        )
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
    pub agents_open: HashSet<String>,
    /// The agent whose install command was copied, and until when.
    pub copied: Option<(String, web_time::Instant)>,
    /// Typed arguments not yet committed, by (agent, field).
    pub args: HashMap<(String, AgentField), String>,
    pub keys: Option<Vec<ConnKey>>,
    pub key_rows: HashMap<String, KeyRowState>,
    pub core_version: Option<String>,
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

/// Why the server will not shut down on its own, if it
/// will not.
pub fn server_stays_up(config: &Value) -> Option<String> {
    let polling = config
        .get("workflows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|w| {
            w.get("enabled").and_then(Value::as_bool) == Some(true)
                && w.get("nodes")
                    .and_then(Value::as_array)
                    .and_then(|n| {
                        n.iter()
                            .find(|n| n.get("type").and_then(Value::as_str) == Some("trigger"))
                    })
                    .and_then(|t| t.pointer("/config/triggerType"))
                    .and_then(Value::as_str)
                    == Some("connectorPoll")
        });
    let network = config
        .pointer("/defaults/networkAccessEnabled")
        .and_then(Value::as_bool)
        == Some(true);
    let reasons: Vec<&str> = [
        network.then_some(
            "Network Access is on, so it stays up to be reached from your other devices",
        ),
        polling.then_some("a connector is polling on a schedule, which it services itself"),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!reasons.is_empty()).then(|| {
        format!(
            "{}. This server will not shut down on its own.",
            reasons.join(", and ")
        )
    })
}

#[cfg(test)]
mod tests {

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

    use super::*;
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
    fn keys_and_holders_say_what_they_hold() {
        let f = KeyFieldInfo {
            key: "k".into(),
            label: "K".into(),
            readable: true,
            hint: None,
            env: Some(vec![]),
        };
        assert_eq!(describe_field(&f), "No variables set");
        assert_eq!(describe_usage(2), "used by 2 steps");
        let h = Holders::from_health(&json!({ "sessiond": { "current": { "pid": 1, "instance": "a" },
            "older": [{ "pid": 2, "instance": "b", "build": "0.7", "sessions": 1, "compatible": true }] } }))
        .expect("holders");
        assert!(h.current);
        assert_eq!(
            older_note(&h.older[0]),
            "1 session started before Vorn was updated is still on the older session holder (0.7). It exits after the last one ends."
        );
    }

    #[test]
    fn the_server_says_why_it_stays_up() {
        assert_eq!(server_stays_up(&json!({ "defaults": {} })), None);
        let c = json!({ "defaults": { "networkAccessEnabled": true }, "workflows": [
            { "enabled": true, "nodes": [{ "type": "trigger", "config": { "triggerType": "connectorPoll" } }] }] });
        assert_eq!(
            server_stays_up(&c).as_deref(),
            Some("Network Access is on, so it stays up to be reached from your other devices, and a connector is polling on a schedule, which it services itself. This server will not shut down on its own.")
        );
        assert_eq!(
            describe_server("0.8", "0.8", false).0,
            "Vorn 0.8, started by this window"
        );
    }
}

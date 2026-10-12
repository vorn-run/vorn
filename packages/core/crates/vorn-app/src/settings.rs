//! The settings page (`SettingsPage` and its panels): which panel is open,
//! what each panel holds that vornd does not, and the wording the panels
//! derive from vornd's answers (`update-status.ts`, `server-runtime.ts`,
//! the worktree manager's totals and selection).

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::Value;

/// A panel of the settings page (`SettingsCategory`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Category {
    #[default]
    Appearance,
    General,
    Updates,
    Notifications,
    Worktrees,
    Agents,
    Ssh,
    Mcp,
    Connectors,
    Keys,
    Network,
    About,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::General => "General",
            Category::Updates => "Updates",
            Category::Notifications => "Notifications",
            Category::Worktrees => "Worktrees",
            Category::Agents => "Coding Agents",
            Category::Ssh => "SSH & Hosts",
            Category::Mcp => "Skills & MCP",
            Category::Connectors => "Plugins",
            Category::Keys => "Keys",
            Category::Network => "Remote Access",
            Category::About => "About",
        }
    }
}

/// The sidebar's sections, as `SIDEBAR_SECTIONS` lists them.
pub const SECTIONS: [(&str, &[Category]); 2] = [
    (
        "APP SETTINGS",
        &[
            Category::Appearance,
            Category::General,
            Category::Updates,
            Category::Notifications,
            Category::Worktrees,
        ],
    ),
    (
        "CONNECTIONS",
        &[
            Category::Agents,
            Category::Ssh,
            Category::Mcp,
            Category::Connectors,
            Category::Keys,
            Category::Network,
        ],
    ),
];

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
    InstallCli,
    CheckUpdates,
    /// The sidebar's update notice put away; the gear keeps a dot.
    DismissUpdate,
    // Worktrees.
    Rescan,
    ToggleWorktree(String),
    SelectSuggested,
    AskReclaim,
    Reclaim,
    AskRemove,
    Remove,
    ClearSelection,
    ExpandBranches(String),
    ToggleBranch(String, String),
    SelectMerged(String),
    DeleteBranches(String),
    // Coding agents.
    Redetect,
    ResetAgent(String),
    ExpandAgent(String),
    CopyInstall(String),
    /// Typing in an agent's command, arguments or headless arguments.
    AgentText(String, AgentField),
    // SSH and hosts.
    AddHost,
    ToggleHost(String),
    HostText(String, HostField),
    Auth(String, String),
    PickKey(String, String),
    Test(String),
    AskDelete(String),
    CancelDelete,
    DeleteHost(String),
    ToggleKey(String),
    DeleteKey(String),
    PasteKey,
    SaveKey,
    CancelKey,
    ImportKey,
    KeyText(KeyField),
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

/// Which of a host's fields a text act edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostField {
    Label,
    Hostname,
    Port,
    User,
    Password,
    KeyPath,
    Options,
}

/// The paste-a-key form's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyField {
    Label,
    Private,
    Public,
    Certificate,
    /// The path Import Key reads.
    Import,
}

/// One of `shell:listInstalled`'s shells (`InstalledShell`).
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

/// The updater's state (`UpdateStatus`). This build has no updater, so it
/// stays `Unsupported`, as an unpackaged desktop build does.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Idle { last_checked: Option<f64> },
    Checking,
    Available { version: String },
    Downloading { version: String, percent: u32 },
    Ready { version: String },
    Error { message: String },
    Unsupported,
}

/// The tones a status can carry (`StatusTone`).
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

/// `formatLastChecked`.
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

/// `describeUpdateStatus`.
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

/// `updateCostLine`: what restarting for an update does to the sessions.
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

/// What the Terminal server row says (`describeServerRuntime`) for a
/// server whose build is known.
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

/// `formatBytes`: binary units, as `du -h` prints them.
pub fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if !bytes.is_finite() || bytes <= 0.0 {
        return "0 B".to_owned();
    }
    let (mut value, mut unit) = (bytes, 0);
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let decimals = if unit <= 1 || value >= 10.0 { 0 } else { 1 };
    format!("{value:.decimals$} {}", UNITS[unit])
}

/// One worktree the manager measured (`WorktreeInventoryEntry`).
#[derive(Debug, Clone, PartialEq)]
pub struct WtEntry {
    pub path: String,
    pub name: String,
    pub project_path: String,
    pub orphan: bool,
    pub branch: Option<String>,
    pub main: bool,
    pub size: f64,
    pub artifacts: f64,
    pub measured: bool,
    pub idle_days: Option<i64>,
    pub dirty: bool,
    pub merged: bool,
    pub active: usize,
    pub level: String,
    pub reasons: Vec<String>,
    pub auto_select: bool,
}

impl WtEntry {
    fn from_value(v: &Value) -> Option<WtEntry> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        let n = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        let verdict = v.get("verdict");
        Some(WtEntry {
            path: s("path")?,
            name: s("name").unwrap_or_default(),
            project_path: s("projectPath").unwrap_or_default(),
            orphan: s("kind").as_deref() == Some("orphan-dir"),
            branch: s("branch"),
            main: b("isMain"),
            size: n("sizeBytes"),
            artifacts: n("artifactBytes"),
            measured: b("sizeMeasured"),
            idle_days: v.get("idleDays").and_then(Value::as_i64),
            dirty: b("isDirty"),
            merged: b("isMerged"),
            active: v
                .get("activeSessionIds")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
            level: verdict
                .and_then(|d| d.get("level"))
                .and_then(Value::as_str)
                .unwrap_or("keep")
                .to_owned(),
            reasons: verdict
                .and_then(|d| d.get("reasons"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|r| r.as_str().map(str::to_owned))
                .collect(),
            auto_select: verdict
                .and_then(|d| d.get("autoSelect"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// `isSelectable`: not the main checkout, and nothing running in it.
    pub fn selectable(&self) -> bool {
        !self.main && self.active == 0
    }

    /// `idleLabel`.
    pub fn idle(&self) -> String {
        match self.idle_days {
            None => "\u{2014}".to_owned(),
            Some(0) => "today".to_owned(),
            Some(d) => format!("{d}d idle"),
        }
    }
}

/// `VERDICT_STYLES`: a verdict's label and its text and fill colours.
pub fn verdict_style(level: &str) -> (&'static str, u32, u32, f32) {
    match level {
        "review" => ("Review", 0xffd230, 0xf99c00, 0.12),
        "reclaim" => ("Build output only", 0x8ec5ff, 0x3080ff, 0.12),
        "remove" => ("Removable", 0x8ec5ff, 0x3080ff, 0.12),
        "orphan" => ("Orphan directory", 0xffa2a2, 0xfb2c36, 0.12),
        _ => ("Keep", 0x99a1af, 0xffffff, 0.05),
    }
}

/// A branch a removed worktree left behind (`StaleBranch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stale {
    pub name: String,
    pub merged: bool,
}

/// One project's worktrees (`WorktreeProjectInventory`).
#[derive(Debug, Clone, PartialEq)]
pub struct WtProject {
    pub path: String,
    pub name: String,
    pub default_branch: Option<String>,
    pub remote: bool,
    pub entries: Vec<WtEntry>,
    pub stale: Vec<Stale>,
    pub error: Option<String>,
}

/// `worktree:inventory`'s projects.
pub fn inventory(v: &Value) -> Vec<WtProject> {
    v.get("projects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_owned);
            Some(WtProject {
                path: s("projectPath")?,
                name: s("projectName").unwrap_or_default(),
                default_branch: s("defaultBranch"),
                remote: p.get("remoteHostId").is_some_and(|r| !r.is_null()),
                entries: p
                    .get("entries")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(WtEntry::from_value)
                    .collect(),
                stale: p
                    .get("staleBranches")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|b| {
                        Some(Stale {
                            name: b.get("name")?.as_str()?.to_owned(),
                            merged: b.get("isMerged").and_then(Value::as_bool).unwrap_or(false),
                        })
                    })
                    .collect(),
                error: s("error"),
            })
        })
        .collect()
}

/// The summary card's three numbers: on disk, build output in selectable
/// worktrees, and removable worktrees.
pub fn totals(projects: &[WtProject]) -> (f64, f64, f64) {
    let (mut disk, mut artifacts, mut removable) = (0.0, 0.0, 0.0);
    for e in projects.iter().flat_map(|p| &p.entries) {
        disk += e.size;
        if e.selectable() {
            artifacts += e.artifacts;
            if e.level == "remove" || e.level == "orphan" {
                removable += e.size;
            }
        }
    }
    (disk, artifacts, removable)
}

/// The worktree manager's page state.
#[derive(Debug, Default)]
pub struct WorktreesPage {
    pub projects: Option<Vec<WtProject>>,
    pub loading: bool,
    pub busy: bool,
    pub selected: BTreeSet<String>,
    /// Stale branches chosen, as (project path, branch).
    pub branches: BTreeSet<(String, String)>,
    /// Projects whose stale-branch strip is open.
    pub expanded: HashSet<String>,
    /// Which destructive action shows its spelled-out second step.
    pub confirming: Option<SetAct>,
}

impl WorktreesPage {
    pub fn entries(&self) -> impl Iterator<Item = &WtEntry> {
        self.projects.iter().flatten().flat_map(|p| &p.entries)
    }

    /// Drops selections the rescan no longer reports.
    pub fn prune(&mut self) {
        let live: HashSet<String> = self.entries().map(|e| e.path.clone()).collect();
        self.selected.retain(|p| live.contains(p));
        let stale: HashSet<(String, String)> = self
            .projects
            .iter()
            .flatten()
            .flat_map(|p| p.stale.iter().map(|b| (p.path.clone(), b.name.clone())))
            .collect();
        self.branches.retain(|b| stale.contains(b));
    }

    pub fn chosen(&self) -> Vec<&WtEntry> {
        self.entries()
            .filter(|e| self.selected.contains(&e.path))
            .collect()
    }

    /// Worktrees with build output to reclaim.
    pub fn reclaim_targets(&self) -> Vec<String> {
        self.entries()
            .filter(|e| e.selectable() && e.artifacts > 0.0)
            .map(|e| e.path.clone())
            .collect()
    }

    pub fn suggested(&self) -> BTreeSet<String> {
        self.entries()
            .filter(|e| e.selectable() && e.auto_select)
            .map(|e| e.path.clone())
            .collect()
    }
}

/// The selection bar's warnings.
pub fn selection_warnings(chosen: &[&WtEntry]) -> Vec<String> {
    let dirty = chosen.iter().filter(|e| e.dirty).count();
    let unmerged = chosen.iter().filter(|e| !e.orphan && !e.merged).count();
    let orphans = chosen.iter().filter(|e| e.orphan).count();
    let mut out = Vec::new();
    if dirty > 0 {
        let verb = if dirty == 1 { "has" } else { "have" };
        out.push(format!(
            "{dirty} {verb} uncommitted changes that will be lost"
        ));
    }
    if unmerged > 0 {
        out.push(format!("{unmerged} not merged \u{2014} the branch is kept"));
    }
    if orphans > 0 {
        out.push(format!("{orphans} deleted from disk directly"));
    }
    out
}

/// `reportResult`: the toasts a batch's answer raises, success first.
pub fn report(result: &Value, verb: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let succeeded = result
        .get("succeeded")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let freed = result
        .get("freedBytes")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if succeeded > 0 {
        let freed = if freed > 0.0 {
            format!(" \u{b7} freed {}", format_bytes(freed))
        } else {
            String::new()
        };
        out.push((format!("{verb} {succeeded}{freed}"), false));
    }
    for f in result
        .get("failed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let path = f.get("path").and_then(Value::as_str).unwrap_or("");
        let name = path.rsplit('/').next().unwrap_or(path);
        let err = f.get("error").and_then(Value::as_str).unwrap_or("");
        out.push((format!("{name}: {err}"), true));
    }
    out
}

/// One of `credential:listKeys`'s stored SSH keys (`SSHKeyMeta`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshKey {
    pub id: String,
    pub label: String,
    pub key_type: Option<String>,
    pub public: Option<String>,
}

impl SshKey {
    pub fn from_value(v: &Value) -> Option<SshKey> {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Some(SshKey {
            id: s("id")?,
            label: s("label").unwrap_or_default(),
            key_type: s("keyType"),
            public: s("publicKey"),
        })
    }
}

/// `truncateKey`: a long public key with its middle cut out.
pub fn truncate_key(key: &str) -> String {
    let n = key.chars().count();
    if n <= 60 {
        return key.to_owned();
    }
    let head: String = key.chars().take(30).collect();
    let tail: String = key.chars().skip(n - 25).collect();
    format!("{head}...{tail}")
}

/// A host's subtitle: `user@host:port`, or what is there of it.
pub fn host_subtitle(host: &Value) -> String {
    let s = |k: &str| host.get(k).and_then(Value::as_str).unwrap_or("");
    let port = host
        .get("port")
        .and_then(Value::as_f64)
        .map_or(22, |p| p as i64);
    let (user, name) = (s("user"), s("hostname"));
    if !user.is_empty() && !name.is_empty() {
        let port = if port == 22 {
            String::new()
        } else {
            format!(":{port}")
        };
        format!("{user}@{name}{port}")
    } else if !name.is_empty() {
        name.to_owned()
    } else {
        "Not configured".to_owned()
    }
}

/// `AUTH_METHODS`.
pub const AUTH_METHODS: [(&str, &str); 4] = [
    ("agent", "SSH Agent"),
    ("password", "Password"),
    ("key-stored", "Stored Key"),
    ("key-file", "Key File"),
];

/// What a host's connection test said: still testing, or ok and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestResult {
    Testing,
    Done { ok: bool, message: String },
}

/// The SSH page's state.
#[derive(Debug, Default)]
pub struct SshPage {
    pub keys: Vec<SshKey>,
    pub hosts_open: HashSet<String>,
    pub keys_open: HashSet<String>,
    pub tests: HashMap<String, TestResult>,
    pub confirm: Option<String>,
    pub adding: bool,
    /// Import Key's typed path, while it is asked for.
    pub importing: bool,
}

/// One of `connection:listKeys`'s entries (`ConnectorKey`).
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

/// `describeField`: what a key's stored value can be said to be.
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

/// `describeUsage`.
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

/// A session holder vornd reports (`SessionHolder`).
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

/// `olderNote`: what an older session holder means for its sessions.
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

/// The `vorn` command's state on this machine (`ShimStatus`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliState {
    pub available: bool,
    pub installed: bool,
    pub path: String,
    pub on_path: bool,
}

/// Everything the settings page holds between frames.
#[derive(Debug, Default)]
pub struct Settings {
    pub open: bool,
    pub category: Category,
    pub shells: Option<Vec<Shell>>,
    /// The shell picker's typed path, while it is shown.
    pub shell_editing: bool,
    pub cli: Option<CliState>,
    pub cli_busy: bool,
    pub cli_error: Option<String>,
    pub update: Option<UpdateStatus>,
    /// The sidebar's ready-update notice was dismissed (`updateBannerDismissed`).
    pub update_dismissed: bool,
    pub worktrees: WorktreesPage,
    pub detecting: bool,
    pub agents_open: HashSet<String>,
    /// The agent whose install command was copied, and until when.
    pub copied: Option<(String, std::time::Instant)>,
    /// Typed arguments not yet committed, by (agent, field).
    pub args: HashMap<(String, AgentField), String>,
    pub ssh: SshPage,
    pub keys: Option<Vec<ConnKey>>,
    pub key_rows: HashMap<String, KeyRowState>,
    pub core_version: Option<String>,
    pub holders: Option<Holders>,
    pub end_failure: Option<String>,
    /// The holder whose End them asks first.
    pub confirm_end: Option<String>,
    /// The Card Row Height slider (`rowHeight`), pixels.
    pub row_height: f32,
    /// The safe-storage check, which the keychain answers here.
    pub safe_storage: bool,
}

impl Settings {
    pub fn new() -> Settings {
        Settings {
            row_height: 208.0,
            safe_storage: true,
            ..Settings::default()
        }
    }
}

/// An agent's definition (`AGENT_DEFINITIONS`) and default command
/// (`DEFAULT_AGENT_COMMANDS`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Agent {
    pub key: &'static str,
    pub name: &'static str,
    pub color: u32,
    pub bg_alpha: f32,
    pub description: &'static str,
    pub install: &'static str,
    pub url: &'static str,
    pub command: &'static str,
    pub headless: &'static [&'static str],
}

pub const AGENTS: [Agent; 5] = [
    Agent {
        key: "claude",
        name: "Claude Code",
        color: 0xd97757,
        bg_alpha: 0x20 as f32 / 255.0,
        description: "Anthropic Claude Code CLI",
        install: "npm install -g @anthropic-ai/claude-code",
        url: "https://docs.anthropic.com/en/docs/claude-code",
        command: "claude",
        headless: &["--dangerously-skip-permissions"],
    },
    Agent {
        key: "copilot",
        name: "GitHub Copilot",
        color: 0x6cc644,
        bg_alpha: 0x20 as f32 / 255.0,
        description: "GitHub Copilot CLI",
        install: "brew install copilot-cli",
        url: "https://docs.github.com/en/copilot/using-github-copilot/using-github-copilot-agent-mode",
        command: "copilot",
        headless: &["--allow-all"],
    },
    Agent {
        key: "codex",
        name: "Codex CLI",
        color: 0x7a9dff,
        bg_alpha: 0x20 as f32 / 255.0,
        description: "OpenAI Codex CLI",
        install: "npm install -g @openai/codex",
        url: "https://github.com/openai/codex",
        command: "codex",
        headless: &["-a", "never"],
    },
    Agent {
        key: "opencode",
        name: "OpenCode",
        color: 0xffffff,
        bg_alpha: 0x15 as f32 / 255.0,
        description: "OpenCode CLI",
        install: "npm install -g opencode-ai",
        url: "https://opencode.ai",
        command: "opencode",
        headless: &[],
    },
    Agent {
        key: "gemini",
        name: "Gemini CLI",
        color: 0x3186ff,
        bg_alpha: 0x20 as f32 / 255.0,
        description: "Google Gemini CLI",
        install: "npm install -g @google/gemini-cli",
        url: "https://github.com/google-gemini/gemini-cli",
        command: "gemini",
        headless: &["-y"],
    },
];

pub fn agent(key: &str) -> Option<&'static Agent> {
    AGENTS.iter().find(|a| a.key == key)
}

/// The default `agentCommands[key]`.
pub fn default_command(a: &Agent) -> Value {
    let mut v = serde_json::json!({ "command": a.command, "args": [] });
    if !a.headless.is_empty() {
        v["headlessArgs"] = serde_json::json!(a.headless);
    }
    v
}

/// `getCommand`: the configured command, else the default.
pub fn command_of(config: &Value, a: &Agent) -> Value {
    config
        .get("agentCommands")
        .and_then(|c| c.get(a.key))
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

/// `isModified`: the command differs from its default.
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

/// `AGENT_MCP_SETUPS`: how each agent connects to Vorn.
pub struct McpSetup {
    pub agent: &'static str,
    pub commands: &'static [&'static str],
    pub in_agent: bool,
    pub note: Option<&'static str>,
}

pub const MCP_SETUPS: [McpSetup; 5] = [
    McpSetup {
        agent: "claude",
        commands: &[
            "/plugin marketplace add vorn-run/plugin",
            "/plugin install vorn",
        ],
        in_agent: true,
        note: None,
    },
    McpSetup {
        agent: "copilot",
        commands: &[
            "copilot plugin marketplace add vorn-run/plugin",
            "copilot plugin install vorn@vorn",
        ],
        in_agent: false,
        note: None,
    },
    McpSetup {
        agent: "codex",
        commands: &[
            "codex plugin marketplace add vorn-run/plugin",
            "codex plugin add vorn@vorn",
        ],
        in_agent: false,
        note: Some("Restart, or start a fresh session \u{2014} skills load at session start."),
    },
    McpSetup {
        agent: "opencode",
        commands: &[
            "opencode mcp add vorn -- npx -y @vornrun/mcp@latest",
            "git clone https://github.com/vorn-run/plugin ~/.config/opencode/vorn",
        ],
        in_agent: false,
        note: Some("Then add \"skills\": { \"paths\": [\"~/.config/opencode/vorn/skills\"] } to ~/.config/opencode/opencode.jsonc"),
    },
    McpSetup {
        agent: "gemini",
        commands: &["gemini extensions install https://github.com/vorn-run/plugin"],
        in_agent: false,
        note: Some(
            "Gemini disables MCP servers in a folder it does not trust, user-level ones included.",
        ),
    },
];

/// `serverStaysUp`: why the server will not shut down on its own, if it
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
    fn bytes_read_as_du_prints_them() {
        assert_eq!(format_bytes(0.0), "0 B");
        assert_eq!(format_bytes(512.0), "512 B");
        assert_eq!(format_bytes(2048.0), "2 KB");
        assert_eq!(format_bytes(1.9 * 1024.0 * 1024.0 * 1024.0), "1.9 GB");
        assert_eq!(format_bytes(12.4 * 1024.0 * 1024.0), "12 MB");
    }

    fn entry(path: &str, level: &str, size: f64, art: f64) -> Value {
        json!({ "path": path, "name": path, "projectPath": "/p", "kind": "registered",
            "branch": "b", "isMain": false, "sizeBytes": size, "artifactBytes": art,
            "sizeMeasured": true, "idleDays": 3, "isDirty": false, "isMerged": true,
            "activeSessionIds": [],
            "verdict": { "level": level, "reasons": ["merged"], "autoSelect": level == "remove" } })
    }

    #[test]
    fn worktree_totals_count_only_what_can_be_selected() {
        let mut busy = entry("/w/b", "remove", 100.0, 40.0);
        busy["activeSessionIds"] = json!(["s"]);
        let inv = json!({ "projects": [{ "projectPath": "/p", "projectName": "p",
            "entries": [entry("/w/a", "remove", 1000.0, 200.0), busy, entry("/w/c", "keep", 50.0, 0.0)],
            "staleBranches": [{ "name": "old", "isMerged": true }] }] });
        let projects = inventory(&inv);
        assert_eq!(totals(&projects), (1150.0, 200.0, 1000.0));
        let mut page = WorktreesPage {
            projects: Some(projects),
            ..WorktreesPage::default()
        };
        assert_eq!(
            page.suggested().into_iter().collect::<Vec<_>>(),
            vec!["/w/a"]
        );
        assert_eq!(page.reclaim_targets(), vec!["/w/a".to_owned()]);
        page.selected.insert("/gone".into());
        page.selected.insert("/w/a".into());
        page.branches.insert(("/p".into(), "gone".into()));
        page.prune();
        assert_eq!(page.selected.len(), 1);
        assert!(page.branches.is_empty());
    }

    #[test]
    fn the_selection_bar_warns_about_what_is_lost() {
        let mut a = WtEntry::from_value(&entry("/w/a", "remove", 1.0, 0.0)).expect("entry");
        a.dirty = true;
        a.merged = false;
        let mut b = a.clone();
        b.dirty = false;
        b.orphan = true;
        assert_eq!(
            selection_warnings(&[&a, &b]),
            vec![
                "1 has uncommitted changes that will be lost".to_owned(),
                "1 not merged \u{2014} the branch is kept".to_owned(),
                "1 deleted from disk directly".to_owned()
            ]
        );
        let r = report(
            &json!({ "succeeded": ["/w/a"], "failed": [{ "path": "/w/b", "error": "busy" }], "freedBytes": 2048 }),
            "Removed",
        );
        assert_eq!(
            r,
            vec![
                ("Removed 1 \u{b7} freed 2 KB".to_owned(), false),
                ("b: busy".to_owned(), true)
            ]
        );
    }

    #[test]
    fn agent_commands_compare_with_their_defaults() {
        let claude = agent("claude").expect("claude");
        let config = json!({});
        let cmd = command_of(&config, claude);
        assert!(!modified(&cmd, claude));
        assert_eq!(args_text(&cmd, true), "--dangerously-skip-permissions");
        let changed =
            json!({ "agentCommands": { "claude": { "command": "claude", "args": ["--x"] } } });
        assert!(modified(&command_of(&changed, claude), claude));
        assert_eq!(split_args("  a   b "), vec!["a", "b"]);
    }

    #[test]
    fn hosts_keys_and_holders_say_what_they_hold() {
        assert_eq!(
            host_subtitle(&json!({ "user": "ci", "hostname": "h", "port": 2222.0 })),
            "ci@h:2222"
        );
        assert_eq!(
            host_subtitle(&json!({ "hostname": "", "port": 22 })),
            "Not configured"
        );
        let long = "x".repeat(80);
        assert_eq!(truncate_key(&long).len(), 58);
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

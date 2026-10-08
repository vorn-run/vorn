//! A pack's `manifest.json`, read into the shape the app draws.
//!
//! The file comes from a package anyone can publish, so every field the app
//! reads is checked here rather than trusted downstream: a bad contribution,
//! icon or sign-in costs only itself, never the pack. These are the SDK's own
//! rules, applied again, so they follow its wording and its limits.

use std::sync::OnceLock;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::js;

/// What the manifest of a pack says it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u64>,
    pub triggers: Vec<Trigger>,
    pub actions: Vec<Action>,
    pub env: Vec<EnvVar>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributes: Option<Contributions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<Permission>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activates: Option<Activation>,
}

/// A connector feeds tasks in; an extension adds to a session's card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Connector,
    Extension,
}

/// A glyph drawn from SVG path data.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Icon {
    pub view_box: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Auth {
    pub rung: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<Probe>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borrow: Option<Borrow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserSignIn>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Probe {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Borrow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSignIn {
    pub sign_in_url: String,
    pub origins: Vec<String>,
    pub check: BrowserCheck,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BrowserCheck {
    pub url: String,
    pub identity: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_mapping: Option<Vec<StatusMapping>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_workflow: Option<DefaultWorkflow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusMapping {
    pub upstream: String,
    pub suggested_local: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultWorkflow {
    pub name: String,
    pub default_cron_from_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnvVar {
    pub name: String,
    pub required: bool,
    pub secret: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Action {
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<ActionInput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<ActionOutput>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionInput {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<ActionOption>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_options: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionOption {
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionOutput {
    pub key: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What an extension adds to a card. Each list is absent rather than empty.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contributions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panes: Option<Vec<Pane>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footers: Option<Vec<Footer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_handlers: Option<Vec<LinkHandler>>,
}

impl Contributions {
    pub fn panes(&self) -> &[Pane] {
        self.panes.as_deref().unwrap_or_default()
    }

    pub fn footers(&self) -> &[Footer] {
        self.footers.as_deref().unwrap_or_default()
    }

    pub fn link_handlers(&self) -> &[LinkHandler] {
        self.link_handlers.as_deref().unwrap_or_default()
    }

    /// Every contribution's own rule, of every kind.
    pub fn rules(&self) -> impl Iterator<Item = Option<&Activation>> {
        let panes = self.panes().iter().map(|p| &p.base);
        let footers = self.footers().iter().map(|f| &f.base);
        let links = self.link_handlers().iter().map(|l| &l.base);
        panes.chain(footers).chain(links).map(|c| c.when.as_ref())
    }
}

/// The id, title and where-it-shows every contribution carries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contribution {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<Activation>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pane {
    #[serde(flatten)]
    pub base: Contribution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    #[serde(flatten)]
    pub draws: PaneDraws,
}

/// A pane is a page inside the pack or a program run in a terminal, never both.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneDraws {
    /// A path under `web/` ending in `.html`.
    Web(String),
    /// Argv, every element non-empty.
    Command(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Footer {
    #[serde(flatten)]
    pub base: Contribution,
    /// Seconds between readings, at least [`MIN_FOOTER_SECONDS`].
    #[serde(serialize_with = "whole_or_fraction")]
    pub every: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkHandler {
    #[serde(flatten)]
    pub base: Contribution,
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

/// Where a contribution shows: every declared field must hold, and any one
/// value of a field satisfies it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activation {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub workspace_contains: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remote_host: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agent: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub platform: Vec<String>,
}

/// What an extension may ask the bridge for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Permission {
    #[serde(rename = "git.read")]
    GitRead,
    #[serde(rename = "terminal.read")]
    TerminalRead,
    #[serde(rename = "terminal.selection")]
    TerminalSelection,
    #[serde(rename = "terminal.send")]
    TerminalSend,
    #[serde(rename = "card.rename")]
    CardRename,
    #[serde(rename = "agent.usage")]
    AgentUsage,
}

impl Permission {
    pub const ALL: [Permission; 6] = [
        Permission::GitRead,
        Permission::TerminalRead,
        Permission::TerminalSelection,
        Permission::TerminalSend,
        Permission::CardRename,
        Permission::AgentUsage,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Permission::GitRead => "git.read",
            Permission::TerminalRead => "terminal.read",
            Permission::TerminalSelection => "terminal.selection",
            Permission::TerminalSend => "terminal.send",
            Permission::CardRename => "card.rename",
            Permission::AgentUsage => "agent.usage",
        }
    }

    fn named(name: &str) -> Option<Permission> {
        Permission::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// Why a manifest describes nothing installable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    MissingIdOrName,
    NothingContributed { name: String },
    NoTriggersOrActions { name: String },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::MissingIdOrName => {
                f.write_str("Connector manifest is missing an id or a name")
            }
            ManifestError::NothingContributed { name } => {
                write!(f, "Extension {name} reports nothing it contributes")
            }
            ManifestError::NoTriggersOrActions { name } => {
                write!(f, "Connector {name} reports no triggers and no actions")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

/// Slowest a footer may be asked for, so a manifest cannot ask for a poll every second.
pub const MIN_FOOTER_SECONDS: f64 = 5.0;
/// Enough of a title or a path to be worth showing.
const MAX_TEXT_LENGTH: usize = 500;
/// Enough contributions to be an extension; past this it is a list nobody reads.
const MAX_CONTRIBUTIONS: usize = 32;
/// Matched against clicked text on a keystroke, so it stays small.
const MAX_PATTERN_LENGTH: usize = 256;
const MAX_ICON_PATHS: usize = 24;
const MAX_PATH_LENGTH: usize = 8_000;
const AUTH_RUNGS: [&str; 5] = ["none", "cli", "key", "browser", "oauth"];
const AGENTS: [&str; 6] = ["claude", "copilot", "codex", "opencode", "gemini", "shell"];
const PLATFORMS: [&str; 3] = ["darwin", "linux", "win32"];
const LOCAL_STATUSES: [&str; 5] = ["todo", "in_progress", "in_review", "done", "cancelled"];

fn whole_or_fraction<S: serde::Serializer>(n: &f64, s: S) -> Result<S::Ok, S::Error> {
    js::json_number(*n).serialize(s)
}

macro_rules! pattern {
    ($name:ident, $source:expr) => {
        fn $name() -> &'static regress::Regex {
            static RE: OnceLock<regress::Regex> = OnceLock::new();
            RE.get_or_init(|| js::regex($source))
        }
    };
}

pattern!(executable_name, r"^[A-Za-z0-9][A-Za-z0-9._-]*$");
pattern!(path_data, r"^[MmZzLlHhVvCcSsQqTtAa0-9\s,.\-+eE]+$");
pattern!(view_box, r"^-?[\d.]+\s+-?[\d.]+\s+-?[\d.]+\s+-?[\d.]+$");
pattern!(web_entry, r"^web\/[A-Za-z0-9._/-]+\.html$");
pattern!(contribution_id, r"^[a-zA-Z][a-zA-Z0-9_-]*$");
pattern!(group_open, r"^\((\?(:|=|!|<=|<!|<[A-Za-z_$][\w$]*>))?");
pattern!(
    origin_pattern,
    r"^https:\/\/(\*\.)?[a-z0-9-]+(\.[a-z0-9-]+)+$"
);
pattern!(header_name, r"^[A-Za-z0-9!#$%&'*+.^_`|~-]+$");

fn record(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

fn strings(raw: Option<&Value>) -> Vec<String> {
    raw.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn text(value: Option<&Value>, fallback: &str) -> String {
    js::slice16(js::str_or(value, fallback), MAX_TEXT_LENGTH).to_owned()
}

fn description(raw: &Map<String, Value>) -> Option<String> {
    raw.get("description")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Reads a manifest the way the SDK's host reads it.
pub fn read(payload: &Map<String, Value>) -> Result<Manifest, ManifestError> {
    let id = js::trim(js::str_of(payload.get("id"))).to_owned();
    let name = js::trim(js::str_of(payload.get("name"))).to_owned();
    if id.is_empty() || name.is_empty() {
        return Err(ManifestError::MissingIdOrName);
    }

    let mut triggers = Vec::new();
    let mut env: Vec<EnvVar> = Vec::new();
    for raw in payload
        .get("triggers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(raw) = raw.as_object() else { continue };
        let kind = js::trim(js::str_of(raw.get("type"))).to_owned();
        if kind.is_empty() {
            continue;
        }
        triggers.push(Trigger {
            label: js::str_or(raw.get("label"), &kind).to_owned(),
            description: description(raw),
            status_mapping: status_mapping(raw.get("statusMapping")),
            default_workflow: default_workflow(raw.get("defaultWorkflow")),
            kind,
        });
        // Every trigger reports the same connector-wide config, so the union is kept.
        let setup = record(raw.get("setup"));
        for entry in setup
            .and_then(|s| s.get("env"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            let var = js::trim(js::str_of(entry.get("name")));
            if var.is_empty() || env.iter().any(|e| e.name == var) {
                continue;
            }
            env.push(EnvVar {
                name: var.to_owned(),
                required: entry.get("required") == Some(&Value::Bool(true)),
                secret: entry.get("secret") == Some(&Value::Bool(true)),
                description: description(entry),
            });
        }
    }

    let actions: Vec<Action> = payload
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|action| {
            let kind = js::str_of(action.get("type")).to_owned();
            Action {
                label: js::str_or(action.get("label"), &kind).to_owned(),
                description: description(action),
                inputs: action.get("inputs").map(|v| action_inputs(Some(v))),
                outputs: action.get("outputs").map(|v| action_outputs(Some(v))),
                kind,
            }
        })
        .filter(|action| !action.kind.is_empty())
        .collect();

    let kind = if payload.get("kind").and_then(Value::as_str) == Some("extension") {
        Kind::Extension
    } else {
        Kind::Connector
    };
    let contributes = match kind {
        Kind::Extension => contributes(payload.get("contributes")),
        Kind::Connector => None,
    };
    if kind == Kind::Extension && contributes.is_none() {
        return Err(ManifestError::NothingContributed { name });
    }
    if kind == Kind::Connector && triggers.is_empty() && actions.is_empty() {
        return Err(ManifestError::NoTriggersOrActions { name });
    }

    let protocol = payload.get("protocol").and_then(protocol);

    Ok(Manifest {
        version: js::str_or(payload.get("version"), "0.0.0").to_owned(),
        description: description(payload),
        icon: icon(payload.get("icon")),
        auth: auth(payload.get("auth")),
        protocol,
        triggers,
        actions,
        env,
        permissions: match kind {
            Kind::Extension => permissions(payload.get("permissions")),
            Kind::Connector => None,
        },
        activates: match kind {
            Kind::Extension => activation(payload.get("activates")),
            Kind::Connector => None,
        },
        contributes,
        kind,
        id,
        name,
    })
}

/// A whole protocol number from 1, which JSON may have written as `1.0`.
/// A newer one is kept, so the app can say the pack needs a newer Vorn.
fn protocol(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return (n >= 1).then_some(n);
    }
    let n = value.as_f64()?;
    // Whole and within u64, checked first.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (n.fract() == 0.0 && (1.0..1.8e19).contains(&n)).then_some(n as u64)
}

fn status_mapping(raw: Option<&Value>) -> Option<Vec<StatusMapping>> {
    let raw = raw?.as_array()?;
    let mapped: Vec<StatusMapping> = raw
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|entry| {
            let upstream = js::trim(js::str_of(entry.get("upstream")));
            let local = js::trim(js::str_of(entry.get("suggestedLocal")));
            (!upstream.is_empty() && LOCAL_STATUSES.contains(&local)).then(|| StatusMapping {
                upstream: upstream.to_owned(),
                suggested_local: local.to_owned(),
            })
        })
        .collect();
    (!mapped.is_empty()).then_some(mapped)
}

fn default_workflow(raw: Option<&Value>) -> Option<DefaultWorkflow> {
    let raw = record(raw)?;
    let name = js::trim(js::str_of(raw.get("name")));
    let minutes = js::number(raw.get("defaultCronFromMinutes"))?;
    if name.is_empty() || minutes.fract() != 0.0 || !(1.0..=1440.0).contains(&minutes) {
        return None;
    }
    // Whole and within 1..=1440, checked above.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let minutes = minutes as u32;
    Some(DefaultWorkflow {
        name: name.to_owned(),
        default_cron_from_minutes: minutes,
    })
}

fn action_inputs(value: Option<&Value>) -> Vec<ActionInput> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|input| !js::str_of(input.get("key")).is_empty())
        .map(|input| {
            let key = js::str_of(input.get("key")).to_owned();
            let options: Vec<ActionOption> = input
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .filter(|o| !js::str_of(o.get("value")).is_empty())
                .map(|o| ActionOption {
                    value: js::str_of(o.get("value")).to_owned(),
                    label: o.get("label").and_then(Value::as_str).map(str::to_owned),
                })
                .collect();
            let load = js::str_of(input.get("loadOptions"));
            ActionInput {
                label: js::str_or(input.get("label"), &key).to_owned(),
                kind: js::str_or(input.get("type"), "string").to_owned(),
                required: input.get("required") == Some(&Value::Bool(true)),
                description: description(input),
                options: (!options.is_empty()).then_some(options),
                load_options: (!load.is_empty()).then(|| load.to_owned()),
                key,
            }
        })
        .collect()
}

fn action_outputs(value: Option<&Value>) -> Vec<ActionOutput> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|entry| {
            let key = entry.get("key").and_then(Value::as_str)?;
            (!key.is_empty()).then(|| ActionOutput {
                key: key.to_owned(),
                kind: entry.get("type").and_then(Value::as_str).map(str::to_owned),
                description: description(entry),
            })
        })
        .collect()
}

/// A malformed icon costs the pack its glyph, nothing more.
fn icon(value: Option<&Value>) -> Option<Icon> {
    let value = record(value)?;
    let paths = value
        .get("paths")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if paths.is_empty() || paths.len() > MAX_ICON_PATHS {
        return None;
    }
    // Dropping only the bad paths would draw a mangled glyph, so one bad path drops the icon.
    let safe: Vec<String> = paths
        .iter()
        .map(|p| {
            p.as_str()
                .filter(|p| js::len16(p) <= MAX_PATH_LENGTH && js::test(path_data(), p))
                .map(str::to_owned)
        })
        .collect::<Option<_>>()?;
    let view = js::trim(js::str_of(value.get("viewBox")));
    Some(Icon {
        view_box: if js::test(view_box(), view) {
            view.to_owned()
        } else {
            "0 0 24 24".to_owned()
        },
        paths: safe,
    })
}

/// An auth block, or nothing when it names a rung this build cannot back.
fn auth(value: Option<&Value>) -> Option<Auth> {
    let value = record(value)?;
    let rung = value.get("rung").and_then(Value::as_str)?;
    if !AUTH_RUNGS.contains(&rung) {
        return None;
    }
    let probe = record(value.get("probe"));
    let command = js::trim(js::str_of(probe.and_then(|p| p.get("command"))));
    let args = probe_args(probe.and_then(|p| p.get("args")));
    let usable = js::test(executable_name(), command) && args.is_some();
    if rung == "cli" && !usable {
        return None;
    }
    let browser = if rung == "browser" {
        Some(browser_sign_in(value.get("browser"))?)
    } else {
        None
    };

    let borrow = record(value.get("borrow"));
    let env = strings(borrow.and_then(|b| b.get("env")));
    let token_args = strings(borrow.and_then(|b| b.get("tokenArgs")));
    let keys = strings(value.get("keys"));
    let asked = js::trim(js::str_of(borrow.and_then(|b| b.get("tokenEnv")))).to_uppercase();
    let token_env = env.iter().find(|n| n.to_uppercase() == asked).cloned();
    let some = |v: Vec<String>| (!v.is_empty()).then_some(v);

    Some(Auth {
        rung: rung.to_owned(),
        probe: match args {
            Some(args) if usable => Some(Probe {
                command: command.to_owned(),
                args: some(args),
            }),
            _ => None,
        },
        borrow: (!env.is_empty() || !token_args.is_empty()).then(|| Borrow {
            env: some(env.clone()),
            token_args: some(token_args),
            token_env,
        }),
        keys: some(keys),
        browser,
    })
}

/// The declared probe arguments, or nothing when any is not a string.
fn probe_args(raw: Option<&Value>) -> Option<Vec<String>> {
    match raw {
        None => Some(Vec::new()),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect::<Option<_>>(),
        Some(_) => None,
    }
}

fn browser_sign_in(value: Option<&Value>) -> Option<BrowserSignIn> {
    let value = record(value)?;
    let check = record(value.get("check"))?;
    let origins: Vec<String> = strings(value.get("origins"))
        .into_iter()
        .filter(|o| origin_ok(o))
        .collect();
    let sign_in_url = js::trim(js::str_of(value.get("signInUrl"))).to_owned();
    let url = js::trim(js::str_of(check.get("url"))).to_owned();
    if !within_origins(&origins, &sign_in_url) || !within_origins(&origins, &url) {
        return None;
    }
    let headers = session_headers(check.get("headers"));
    Some(BrowserSignIn {
        sign_in_url,
        origins,
        check: BrowserCheck {
            url,
            identity: strings(check.get("identity")),
            headers: (!headers.is_empty()).then_some(headers),
        },
    })
}

/// `https://host` or `https://*.host`, case aside.
fn origin_ok(origin: &str) -> bool {
    js::test(origin_pattern(), &origin.to_ascii_lowercase())
}

/// Whether `url` is https on its default port and inside one of `origins`.
pub fn within_origins(origins: &[String], url: &str) -> bool {
    let Some(target) = https_host(url) else {
        return false;
    };
    origins.iter().any(|origin| {
        if !origin_ok(origin) {
            return false;
        }
        let wildcard = origin.starts_with("https://*.");
        let skip = if wildcard {
            "https://*.".len()
        } else {
            "https://".len()
        };
        let host = origin[skip..].to_ascii_lowercase();
        if wildcard {
            target.ends_with(&format!(".{host}"))
        } else {
            target == host
        }
    })
}

/// The lowercase host of an `https:` URL on its default port, or `None`.
///
/// No URL crate is in the build, and this is all `withinOrigins` reads: a
/// URL with an explicit port other than 443 or with no host is refused.
fn https_host(url: &str) -> Option<String> {
    let rest = url
        .get(..8)
        .filter(|scheme| scheme.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (h, Some(p)),
        _ => (host_port, None),
    };
    match port {
        None | Some("" | "443") => {}
        Some(_) => return None,
    }
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// Headers a sign-in check may send: never a credential or a routing header.
fn session_headers(value: Option<&Value>) -> Map<String, Value> {
    const FORBIDDEN: [&str; 7] = [
        "cookie",
        "cookie2",
        "authorization",
        "host",
        "origin",
        "referer",
        "content-length",
    ];
    let Some(value) = record(value) else {
        return Map::new();
    };
    value
        .iter()
        .filter(|(name, v)| {
            let lower = name.to_ascii_lowercase();
            v.is_string()
                && js::test(header_name(), name)
                && !FORBIDDEN.contains(&lower.as_str())
                && !lower.starts_with("sec-")
                && !lower.starts_with("proxy-")
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Permissions this build can enforce, once each, in the order asked.
fn permissions(value: Option<&Value>) -> Option<Vec<Permission>> {
    let raw = value?.as_array()?;
    let mut kept = Vec::new();
    for p in raw
        .iter()
        .filter_map(Value::as_str)
        .filter_map(Permission::named)
    {
        if !kept.contains(&p) {
            kept.push(p);
        }
    }
    Some(kept)
}

/// Up to 32 strings, each cut to 500 units, trimmed, blanks dropped.
fn bounded_names(raw: Option<&Value>) -> Vec<String> {
    strings(raw)
        .iter()
        .take(MAX_CONTRIBUTIONS)
        .map(|s| js::trim(js::slice16(s, MAX_TEXT_LENGTH)).to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Where a contribution shows. A predicate this build cannot evaluate is
/// dropped, which widens where it shows rather than hiding it.
pub fn activation(value: Option<&Value>) -> Option<Activation> {
    let value = record(value)?;
    let activation = Activation {
        workspace_contains: bounded_names(value.get("workspaceContains"))
            .into_iter()
            .filter(|g| !g.starts_with('/') && !g.split('/').any(|s| s == ".."))
            .collect(),
        remote_host: bounded_names(value.get("remoteHost")),
        agent: bounded_names(value.get("agent"))
            .into_iter()
            .filter(|a| AGENTS.contains(&a.as_str()))
            .collect(),
        platform: bounded_names(value.get("platform"))
            .into_iter()
            .filter(|p| PLATFORMS.contains(&p.as_str()))
            .collect(),
    };
    (activation != Activation::default()).then_some(activation)
}

fn contribution(raw: &Map<String, Value>) -> Option<Contribution> {
    let id = js::trim(js::str_of(raw.get("id")));
    if id.is_empty() || !js::test(contribution_id(), id) {
        return None;
    }
    Some(Contribution {
        title: text(raw.get("title"), id),
        description: raw
            .get("description")
            .and_then(Value::as_str)
            .map(|d| js::slice16(d, MAX_TEXT_LENGTH).to_owned()),
        when: activation(raw.get("when")),
        id: id.to_owned(),
    })
}

/// What an extension contributes, keeping only what the app could draw.
pub fn contributes(value: Option<&Value>) -> Option<Contributions> {
    let value = record(value)?;
    let declared = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_array)
            .map(|a| &a[..a.len().min(MAX_CONTRIBUTIONS)])
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|raw| Some((contribution(raw)?, raw)))
    };

    let panes: Vec<Pane> = declared("panes")
        .filter_map(|(base, raw)| {
            let web = js::trim(&text(raw.get("web"), "")).to_owned();
            let command: Vec<String> = strings(raw.get("command"))
                .iter()
                .take(MAX_CONTRIBUTIONS)
                .map(|a| js::slice16(a, MAX_TEXT_LENGTH).to_owned())
                .collect();
            let draws = if !web.is_empty()
                && js::test(web_entry(), &web)
                && !web.split('/').any(|s| s == "..")
            {
                PaneDraws::Web(web)
            } else if !command.is_empty() && command.iter().all(|a| !a.is_empty()) {
                PaneDraws::Command(command)
            } else {
                return None;
            };
            Some(Pane {
                base,
                icon: icon(raw.get("icon")),
                draws,
            })
        })
        .collect();

    let footers: Vec<Footer> = declared("footers")
        .filter_map(|(base, raw)| {
            let every = js::number(raw.get("every"))?;
            (every.is_finite() && every >= MIN_FOOTER_SECONDS).then_some(Footer { base, every })
        })
        .collect();

    let link_handlers: Vec<LinkHandler> = declared("linkHandlers")
        .filter_map(|(base, raw)| {
            let pattern = js::str_of(raw.get("pattern"));
            if pattern.is_empty()
                || js::len16(pattern) > MAX_PATTERN_LENGTH
                || has_nested_quantifier(pattern)
                || regress::Regex::new(pattern).is_err()
            {
                return None;
            }
            let example = js::trim(&text(raw.get("example"), "")).to_owned();
            Some(LinkHandler {
                base,
                pattern: pattern.to_owned(),
                example: (!example.is_empty()).then_some(example),
            })
        })
        .collect();

    if panes.is_empty() && footers.is_empty() && link_handlers.is_empty() {
        return None;
    }
    Some(Contributions {
        panes: (!panes.is_empty()).then_some(panes),
        footers: (!footers.is_empty()).then_some(footers),
        link_handlers: (!link_handlers.is_empty()).then_some(link_handlers),
    })
}

/// Whether a quantifier is applied to a group that already holds one, such
/// as `(a+)+`, which costs exponential time on text that nearly fits.
fn has_nested_quantifier(pattern: &str) -> bool {
    // Indexed in UTF-16 units, as the SDK walks it.
    let units: Vec<u16> = pattern.encode_utf16().collect();
    let quantifier_at = |at: usize| {
        units
            .get(at)
            .is_some_and(|&u| b"*+?{".iter().any(|&q| u16::from(q) == u))
    };
    let is = |at: usize, ch: u8| units[at] == u16::from(ch);
    let mut quantified: Vec<bool> = Vec::new();
    let mut in_class = false;
    let mut i = 0;
    while i < units.len() {
        if is(i, b'\\') {
            i += 2;
            continue;
        }
        if in_class {
            if is(i, b']') {
                in_class = false;
            }
            i += 1;
            continue;
        }
        if is(i, b'[') {
            in_class = true;
        } else if is(i, b'(') {
            quantified.push(false);
            let rest = String::from_utf16_lossy(&units[i..]);
            let open = group_open()
                .find(&rest)
                .map_or(1, |m| js::len16(&rest[m.range()]));
            i += open;
            continue;
        } else if is(i, b')') {
            let held = quantified.pop().unwrap_or(false);
            let repeated = quantifier_at(i + 1);
            if held && repeated {
                return true;
            }
            if held || repeated {
                if let Some(parent) = quantified.last_mut() {
                    *parent = true;
                }
            }
        } else if quantifier_at(i) {
            if let Some(last) = quantified.last_mut() {
                *last = true;
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(v: Value) -> Result<Manifest, ManifestError> {
        read(v.as_object().unwrap())
    }

    fn extension(contributes: Value) -> Value {
        json!({ "id": "x", "name": "X", "kind": "extension", "contributes": contributes })
    }

    #[test]
    fn needs_an_id_and_a_name() {
        assert_eq!(
            manifest(json!({ "id": " ", "name": "n" }))
                .unwrap_err()
                .to_string(),
            "Connector manifest is missing an id or a name"
        );
    }

    #[test]
    fn needs_something_to_do() {
        assert_eq!(
            manifest(json!({ "id": "a", "name": "A" }))
                .unwrap_err()
                .to_string(),
            "Connector A reports no triggers and no actions"
        );
        assert_eq!(
            manifest(extension(json!({ "panes": [] })))
                .unwrap_err()
                .to_string(),
            "Extension X reports nothing it contributes"
        );
    }

    #[test]
    fn reads_a_connector_as_the_sdk_host_does() {
        let m = manifest(json!({
            "id": " jira ", "name": "Jira", "protocol": 1.0, "description": "d",
            "triggers": [
                { "type": "issue", "statusMapping": [{ "upstream": "Open", "suggestedLocal": "todo" }, { "upstream": "x", "suggestedLocal": "nope" }],
                  "defaultWorkflow": { "name": "w", "defaultCronFromMinutes": 15 },
                  "setup": { "env": [{ "name": "TOKEN", "secret": true }, { "name": "TOKEN" }] } },
                { "type": "" }, 7
            ],
            "actions": [{ "type": "comment", "inputs": [{ "key": "k", "options": [{ "value": "" }, { "value": "a" }] }, { "label": "nokey" }], "outputs": [{ "key": "" }, { "key": "id", "type": "string" }] }, { "label": "untyped" }],
            "icon": { "paths": ["M0 0L1 1"], "viewBox": "bad" }
        }))
        .unwrap();
        let out = serde_json::to_value(&m).unwrap();
        assert_eq!(out["id"], "jira");
        assert_eq!(out["version"], "0.0.0");
        assert_eq!(out["protocol"], 1);
        assert_eq!(out["kind"], "connector");
        assert_eq!(
            out["triggers"],
            json!([{ "type": "issue", "label": "issue", "statusMapping": [{ "upstream": "Open", "suggestedLocal": "todo" }], "defaultWorkflow": { "name": "w", "defaultCronFromMinutes": 15 } }])
        );
        assert_eq!(
            out["env"],
            json!([{ "name": "TOKEN", "required": false, "secret": true }])
        );
        assert_eq!(
            out["actions"],
            json!([{ "type": "comment", "label": "comment", "inputs": [{ "key": "k", "label": "k", "type": "string", "required": false, "options": [{ "value": "a" }] }], "outputs": [{ "key": "id", "type": "string" }] }])
        );
        assert_eq!(
            out["icon"],
            json!({ "viewBox": "0 0 24 24", "paths": ["M0 0L1 1"] })
        );
        assert!(out.get("contributes").is_none());
    }

    #[test]
    fn drops_an_icon_with_one_bad_path() {
        assert_eq!(icon(Some(&json!({ "paths": ["M0 0", "<script>"] }))), None);
        assert_eq!(icon(Some(&json!({ "paths": [] }))), None);
    }

    #[test]
    fn keeps_an_auth_rung_only_when_it_is_backed() {
        assert_eq!(
            auth(Some(
                &json!({ "rung": "cli", "probe": { "command": "gh; rm" } })
            )),
            None
        );
        assert_eq!(auth(Some(&json!({ "rung": "magic" }))), None);
        let cli = auth(Some(&json!({
            "rung": "cli", "probe": { "command": "gh", "args": ["auth", "status"] },
            "borrow": { "env": ["GH_TOKEN"], "tokenEnv": "gh_token" }, "keys": ["k"]
        })))
        .unwrap();
        assert_eq!(
            serde_json::to_value(cli).unwrap(),
            json!({ "rung": "cli", "probe": { "command": "gh", "args": ["auth", "status"] }, "borrow": { "env": ["GH_TOKEN"], "tokenEnv": "GH_TOKEN" }, "keys": ["k"] })
        );
        let browser = |sign_in: &str| {
            auth(Some(&json!({ "rung": "browser", "browser": {
                "signInUrl": sign_in, "origins": ["https://*.example.com", "http://bad.com"],
                "check": { "url": "https://api.example.com/me", "identity": ["login"], "headers": { "X-Ok": "1", "Cookie": "no", "sec-x": "no" } }
            } })))
        };
        let ok = browser("https://login.example.com/start").unwrap();
        assert_eq!(
            serde_json::to_value(ok).unwrap()["browser"],
            json!({ "signInUrl": "https://login.example.com/start", "origins": ["https://*.example.com"], "check": { "url": "https://api.example.com/me", "identity": ["login"], "headers": { "X-Ok": "1" } } })
        );
        assert_eq!(browser("https://login.example.com:8443/"), None);
        assert_eq!(browser("https://evil.com/"), None);
        assert_eq!(browser("http://login.example.com/"), None);
    }

    #[test]
    fn reads_what_an_extension_contributes() {
        let m = manifest(json!({
            "id": "x", "name": "X", "kind": "extension", "version": "1.2.0",
            "permissions": ["git.read", "git.read", "root", "agent.usage"],
            "activates": { "agent": ["claude", "vim"], "workspaceContains": ["/etc", "a/../b", "package.json"] },
            "contributes": {
                "panes": [
                    { "id": "page", "web": "web/index.html", "command": ["x"] },
                    { "id": "prog", "title": "Program", "command": ["lazygit", "-p"] },
                    { "id": "out", "web": "web/../x.html" },
                    { "id": "blank", "command": ["a", ""] },
                    { "id": "9bad", "web": "web/a.html" }
                ],
                "footers": [{ "id": "f", "every": 4 }, { "id": "g", "every": "10", "when": { "platform": ["darwin", "beos"] } }],
                "linkHandlers": [
                    { "id": "l", "pattern": "PROJ-\\d+", "example": "  PROJ-1 " },
                    { "id": "n", "pattern": "(a+)+" },
                    { "id": "c", "pattern": "(" }
                ]
            }
        }))
        .unwrap();
        let out = serde_json::to_value(&m).unwrap();
        assert_eq!(out["permissions"], json!(["git.read", "agent.usage"]));
        assert_eq!(
            out["activates"],
            json!({ "workspaceContains": ["package.json"], "agent": ["claude"] })
        );
        assert_eq!(
            out["contributes"],
            json!({
                "panes": [
                    { "id": "page", "title": "page", "web": "web/index.html" },
                    { "id": "prog", "title": "Program", "command": ["lazygit", "-p"] }
                ],
                "footers": [{ "id": "g", "title": "g", "when": { "platform": ["darwin"] }, "every": 10 }],
                "linkHandlers": [{ "id": "l", "title": "l", "pattern": "PROJ-\\d+", "example": "PROJ-1" }]
            })
        );
    }

    #[test]
    fn finds_a_quantifier_on_a_group_that_holds_one() {
        for nested in [
            "(a+)+",
            "(a*)*",
            "((a)+)*",
            "(?:a|b+){2}",
            "(?<n>a+)+",
            "((a+))+",
        ] {
            assert!(has_nested_quantifier(nested), "{nested}");
        }
        for flat in [
            "a+b+",
            "(ab)+",
            "[(+]+",
            "\\(a+\\)+",
            "(?:ab)?c",
            "(a)(b+)",
            "(",
        ] {
            assert!(!has_nested_quantifier(flat), "{flat}");
        }
    }

    #[test]
    fn reads_https_hosts_only_on_the_default_port() {
        assert_eq!(
            https_host("https://A.example.com/x").as_deref(),
            Some("a.example.com")
        );
        assert_eq!(
            https_host("HTTPS://u:p@a.com:443?q").as_deref(),
            Some("a.com")
        );
        assert_eq!(https_host("https://a.com:8443/"), None);
        assert_eq!(https_host("https:///x"), None);
        assert_eq!(https_host("ftp://a.com"), None);
    }
}

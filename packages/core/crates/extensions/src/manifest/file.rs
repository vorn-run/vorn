//! `vorn-extension.toml`, the manifest a package's author writes.
//!
//! Parsing is strict where JSON reading is lenient: a misspelt key is an
//! error rather than a feature silently off, and every error names the file,
//! the line and the field. The parts 0.8 packs already carried (a
//! connector's triggers and actions, an extension's contributions) then go
//! through the same checks [`super::read`] applies to `manifest.json`, so a
//! value the app would drop is reported instead.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml::de::{DeTable, DeValue};
use toml::Spanned;

use super::{
    Credential, Declares, Manifest, ManifestError, Permission, Run, Service, API, CONNECTOR_API,
};
use crate::connector::SUPPORTED_PROTOCOLS;

/// The manifest's file name in a package's folder.
pub const FILE: &str = "vorn-extension.toml";
/// What a 0.8 pack ran: `node index.js` in its folder.
pub const PACK_ENTRY: [&str; 2] = ["node", "index.js"];

/// A manifest that does not parse or check, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    pub line: usize,
    /// The dotted path of the field, empty when the error is the file's.
    pub field: String,
    pub message: String,
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.field.is_empty() {
            write!(f, "{FILE}:{}: {}", self.line, self.message)
        } else {
            write!(f, "{FILE}:{}: {}: {}", self.line, self.field, self.message)
        }
    }
}

impl std::error::Error for FileError {}

/// Reads and checks a manifest's TOML text.
pub fn parse(text: &str) -> Result<Manifest, FileError> {
    let lines = Lines::parse(text)?;
    let raw: Raw = toml::from_str(text).map_err(|e| FileError {
        line: e.span().map_or(1, |s| line_of(text, s.start)),
        field: String::new(),
        message: e.message().trim_end().to_owned(),
    })?;
    let at = |field: &str, message: String| FileError {
        line: lines.line(field),
        field: field.to_owned(),
        message,
    };
    raw.check(&at)?;
    let manifest = raw.into_manifest(&at)?;
    Ok(manifest)
}

/// A 0.8 pack's `manifest.json`, read as its folder runs it: `node index.js`.
pub fn from_pack_json(payload: &Map<String, Value>) -> Result<Manifest, ManifestError> {
    let mut manifest = super::read(payload)?;
    manifest.service = Some(Service {
        command: PACK_ENTRY.map(str::to_owned).to_vec(),
        events: Vec::new(),
    });
    Ok(manifest)
}

/// The manifest as `vorn-extension.toml` writes it.
pub fn to_toml(manifest: &Manifest) -> String {
    let raw = Raw::from(manifest);
    toml::to_string_pretty(&raw).unwrap_or_default()
}

// ---- the file as written ----

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    id: String,
    name: String,
    version: String,
    api: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vorn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<RawIcon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    build: Option<RawCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    service: Option<RawService>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    actions: BTreeMap<String, RawRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checks: Option<RawCommand>,
    #[serde(default, skip_serializing_if = "RawDeclares::is_empty")]
    declares: RawDeclares,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential: Option<RawCredential>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connector: Option<RawConnector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activates: Option<RawActivation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contributes: Option<RawContributes>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawIcon {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    view_box: Option<String>,
    paths: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    command: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawService {
    command: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    events: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawRun {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    command: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawDeclares {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    network: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    uses: Vec<String>,
}

impl RawDeclares {
    fn is_empty(&self) -> bool {
        self.network.is_empty() && self.uses.is_empty()
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawCredential {
    field: String,
    header: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawConnector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth: Option<RawAuth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    triggers: Vec<RawTrigger>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    actions: Vec<RawAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    env: Vec<RawEnv>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawAuth {
    rung: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    probe: Option<RawProbe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    borrow: Option<RawBorrow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keys: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    browser: Option<RawBrowser>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawProbe {
    command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    args: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawBorrow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    env: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_env: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawBrowser {
    sign_in_url: String,
    origins: Vec<String>,
    check: RawCheck,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawCheck {
    url: String,
    #[serde(default)]
    identity: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawTrigger {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status_mapping: Option<Vec<RawStatusMapping>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_workflow: Option<RawDefaultWorkflow>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawStatusMapping {
    upstream: String,
    suggested_local: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawDefaultWorkflow {
    name: String,
    default_cron_from_minutes: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inputs: Option<Vec<RawInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outputs: Option<Vec<RawOutput>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawInput {
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    options: Option<Vec<RawOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    load_options: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawOption {
    value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawOutput {
    key: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawEnv {
    name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    required: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawActivation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workspace_contains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    remote_host: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    agent: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    platform: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawContributes {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    panes: Vec<RawPane>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    footers: Vec<RawFooter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    link_handlers: Vec<RawLink>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawPane {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    when: Option<RawActivation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<RawIcon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    web: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawFooter {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    when: Option<RawActivation>,
    every: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawLink {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    when: Option<RawActivation>,
    pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    example: Option<String>,
}

// ---- checks ----

type At<'a> = dyn Fn(&str, String) -> FileError + 'a;

fn is_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-._".contains(c))
        && !id.contains("..")
}

fn is_word(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphabetic()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_')
}

/// `api.example.com` or `*.example.com`, lowercase; `localhost` too.
pub fn is_host(host: &str) -> bool {
    let name = host.strip_prefix("*.").unwrap_or(host);
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && !label.starts_with('-')
        })
}

/// An event name: `session.exited`, `git.changed`.
fn is_event(name: &str) -> bool {
    let mut parts = name.split('.');
    let first = parts.next().unwrap_or_default();
    is_word(first) && parts.clone().count() >= 1 && parts.all(is_word)
}

fn check_command(at: &At<'_>, field: &str, argv: &[String]) -> Result<(), FileError> {
    if argv.is_empty() {
        return Err(at(field, "is empty; it names the program to run".into()));
    }
    if let Some(i) = argv.iter().position(|a| a.trim().is_empty()) {
        return Err(at(&format!("{field}[{i}]"), "is blank".into()));
    }
    Ok(())
}

/// A version range of comparators such as `>=0.9, <2`; checked for its form only.
pub fn is_range(range: &str) -> bool {
    range.split(',').map(str::trim).all(|part| {
        let version = part.trim_start_matches(['>', '<', '=', '^', '~']);
        let op = &part[..part.len() - version.len()];
        matches!(op, "" | ">=" | ">" | "<=" | "<" | "=" | "^" | "~")
            && !version.is_empty()
            && version.split(['.', '-', '+']).all(|p| {
                !p.is_empty() && p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'*')
            })
            && version
                .split('.')
                .next()
                .is_some_and(|major| major.bytes().all(|c| c.is_ascii_digit()))
    })
}

impl Raw {
    fn check(&self, at: &At<'_>) -> Result<(), FileError> {
        if !is_id(&self.id) {
            return Err(at(
                "id",
                format!(
                    "`{}` may hold only a-z, 0-9, `-`, `.` and `_`, 64 at most",
                    self.id
                ),
            ));
        }
        if self.name.trim().is_empty() {
            return Err(at("name", "is blank".into()));
        }
        if !crate::pack::is_safe_version(&self.version) {
            return Err(at(
                "version",
                format!("`{}` is not a version like 1.2.0", self.version),
            ));
        }
        let connector_protocol = self.api.strip_prefix(CONNECTOR_API);
        let speaks = self.api == API
            || connector_protocol
                .and_then(|p| p.parse::<u64>().ok())
                .is_some_and(|p| SUPPORTED_PROTOCOLS.contains(&p));
        if !speaks {
            return Err(at(
                "api",
                format!(
                    "this Vorn speaks {API} and {CONNECTOR_API}1; the package asks for {}",
                    self.api
                ),
            ));
        }
        if let Some(range) = &self.vorn {
            if !is_range(range) {
                return Err(at("vorn", format!("`{range}` is not a range like >=0.9")));
            }
        }
        if let Some(build) = &self.build {
            check_command(at, "build.command", &build.command)?;
        }
        if let Some(checks) = &self.checks {
            check_command(at, "checks.command", &checks.command)?;
        }
        if let Some(service) = &self.service {
            check_command(at, "service.command", &service.command)?;
            for (i, event) in service.events.iter().enumerate() {
                if !is_event(event) {
                    return Err(at(
                        &format!("service.events[{i}]"),
                        format!("`{event}` is not an event name like session.exited"),
                    ));
                }
            }
        }
        for (id, run) in &self.actions {
            if !is_word(id) {
                return Err(at(
                    &format!("actions.{id}"),
                    "an action id starts with a letter and holds letters, digits, `-` and `_`"
                        .into(),
                ));
            }
            check_command(at, &format!("actions.{id}.command"), &run.command)?;
        }
        for (i, host) in self.declares.network.iter().enumerate() {
            if !is_host(host) {
                return Err(at(
                    &format!("declares.network[{i}]"),
                    format!("`{host}` is not a host name like api.example.com"),
                ));
            }
        }
        for (i, used) in self.declares.uses.iter().enumerate() {
            if Permission::named(used).is_none() {
                return Err(at(
                    &format!("declares.uses[{i}]"),
                    format!("unknown permission `{used}`"),
                ));
            }
        }
        if let Some(credential) = &self.credential {
            if credential.field.trim().is_empty() {
                return Err(at("credential.field", "is blank".into()));
            }
            let named = credential
                .header
                .split_once(':')
                .is_some_and(|(n, v)| !n.trim().is_empty() && v.contains("{token}"));
            if !named {
                return Err(at(
                    "credential.header",
                    "is not a header like `Authorization: Bearer {token}`".into(),
                ));
            }
            if self.declares.network.is_empty() {
                return Err(at(
                    "credential",
                    "is added only to declared hosts, and declares.network names none".into(),
                ));
            }
        }
        let legacy = self.connector.is_some() || self.contributes.is_some();
        if legacy && connector_protocol.is_none() {
            let table = if self.connector.is_some() {
                "connector"
            } else {
                "contributes"
            };
            return Err(at(
                table,
                format!("is spoken to in the connector protocol; set api = \"{CONNECTOR_API}1\""),
            ));
        }
        let does_something = self.service.is_some()
            || !self.actions.is_empty()
            || self
                .connector
                .as_ref()
                .is_some_and(|c| !c.triggers.is_empty() || !c.actions.is_empty())
            || self.contributes.is_some();
        if !does_something {
            return Err(at(
                "",
                "declares no service, action, connector or contribution".into(),
            ));
        }
        Ok(())
    }

    fn into_manifest(self, at: &At<'_>) -> Result<Manifest, FileError> {
        let protocol = self
            .api
            .strip_prefix(CONNECTOR_API)
            .and_then(|p| p.parse::<u64>().ok());
        let mut payload = Map::new();
        payload.insert("id".into(), json!(self.id));
        payload.insert("name".into(), json!(self.name));
        payload.insert("version".into(), json!(self.version));
        if let Some(d) = &self.description {
            payload.insert("description".into(), json!(d));
        }
        if let Some(icon) = &self.icon {
            let value = icon.to_json();
            if super::icon(Some(&value)).is_none() {
                return Err(at("icon", "has a path that is not SVG path data".into()));
            }
            payload.insert("icon".into(), value);
        }
        if let Some(c) = &self.connector {
            if let Some(auth) = &c.auth {
                let value = auth.to_json();
                if super::auth(Some(&value)).is_none() {
                    return Err(at(
                        "connector.auth",
                        format!("names a `{}` sign-in this Vorn cannot back", auth.rung),
                    ));
                }
                payload.insert("auth".into(), value);
            }
            let mut triggers = Vec::new();
            for (i, t) in c.triggers.iter().enumerate() {
                triggers.push(t.to_json(at, i)?);
            }
            payload.insert("triggers".into(), Value::Array(triggers));
            payload.insert(
                "actions".into(),
                Value::Array(c.actions.iter().map(RawAction::to_json).collect()),
            );
            payload.insert(
                "env".into(),
                Value::Array(c.env.iter().map(RawEnv::to_json).collect()),
            );
        }
        // A program action is enough for the JSON reader's "something to do".
        let has_connector_work = payload
            .get("triggers")
            .and_then(Value::as_array)
            .is_some_and(|t| !t.is_empty())
            || payload
                .get("actions")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty());
        if let Some(contributes) = &self.contributes {
            payload.insert("kind".into(), json!("extension"));
            payload.insert("contributes".into(), contributes.to_json(at)?);
        } else if !has_connector_work {
            payload.insert("actions".into(), json!([{ "type": "_" }]));
        }
        if let Some(activates) = &self.activates {
            let value = activates.to_json();
            check_activation(at, "activates", activates, &value)?;
            payload.insert("activates".into(), value);
        }
        let uses: Vec<Value> = self.declares.uses.iter().map(|u| json!(u)).collect();
        payload.insert("permissions".into(), Value::Array(uses));
        if let Some(p) = protocol {
            payload.insert("protocol".into(), json!(p));
        }
        let mut manifest = super::read(&payload).map_err(|e| at("", e.to_string()))?;
        if !has_connector_work {
            manifest.actions.clear();
        }
        if self.contributes.is_none() {
            // Only a package that adds to a card reads `permissions` in JSON.
            let mut kept: Vec<Permission> = Vec::new();
            for p in self
                .declares
                .uses
                .iter()
                .filter_map(|u| Permission::named(u))
            {
                if !kept.contains(&p) {
                    kept.push(p);
                }
            }
            manifest.permissions = (!kept.is_empty()).then_some(kept);
        }
        manifest.api = Some(self.api);
        manifest.protocol = protocol;
        manifest.vorn = self.vorn;
        manifest.publisher = self.publisher;
        manifest.build = self.build.map(|b| b.command);
        manifest.service = self.service.map(|s| Service {
            command: s.command,
            events: s.events,
        });
        manifest.runs = self
            .actions
            .into_iter()
            .map(|(id, run)| Run {
                title: run.title.unwrap_or_else(|| id.clone()),
                command: run.command,
                id,
            })
            .collect();
        manifest.checks = self.checks.map(|c| c.command);
        manifest.declares = Declares {
            network: self.declares.network,
            uses: self.declares.uses,
        };
        manifest.credential = self.credential.map(|c| Credential {
            field: c.field,
            header: c.header,
        });
        Ok(manifest)
    }
}

fn opt(map: &mut Map<String, Value>, key: &str, value: Option<&String>) {
    if let Some(v) = value {
        map.insert(key.into(), json!(v));
    }
}

impl RawIcon {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        opt(&mut m, "viewBox", self.view_box.as_ref());
        m.insert("paths".into(), json!(self.paths));
        Value::Object(m)
    }
}

impl RawAuth {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("rung".into(), json!(self.rung));
        if let Some(p) = &self.probe {
            let mut probe = Map::new();
            probe.insert("command".into(), json!(p.command));
            if let Some(args) = &p.args {
                probe.insert("args".into(), json!(args));
            }
            m.insert("probe".into(), Value::Object(probe));
        }
        if let Some(b) = &self.borrow {
            let mut borrow = Map::new();
            if let Some(env) = &b.env {
                borrow.insert("env".into(), json!(env));
            }
            if let Some(args) = &b.token_args {
                borrow.insert("tokenArgs".into(), json!(args));
            }
            opt(&mut borrow, "tokenEnv", b.token_env.as_ref());
            m.insert("borrow".into(), Value::Object(borrow));
        }
        if let Some(keys) = &self.keys {
            m.insert("keys".into(), json!(keys));
        }
        if let Some(b) = &self.browser {
            let mut check = Map::new();
            check.insert("url".into(), json!(b.check.url));
            check.insert("identity".into(), json!(b.check.identity));
            if let Some(h) = &b.check.headers {
                check.insert("headers".into(), json!(h));
            }
            m.insert(
                "browser".into(),
                json!({ "signInUrl": b.sign_in_url, "origins": b.origins, "check": check }),
            );
        }
        Value::Object(m)
    }
}

impl RawTrigger {
    fn to_json(&self, at: &At<'_>, i: usize) -> Result<Value, FileError> {
        let field = format!("connector.triggers[{i}]");
        if self.kind.trim().is_empty() {
            return Err(at(&format!("{field}.type"), "is blank".into()));
        }
        let mut m = Map::new();
        m.insert("type".into(), json!(self.kind));
        opt(&mut m, "label", self.label.as_ref());
        opt(&mut m, "description", self.description.as_ref());
        if let Some(mapping) = &self.status_mapping {
            let entries: Vec<Value> = mapping
                .iter()
                .map(|s| json!({ "upstream": s.upstream, "suggestedLocal": s.suggested_local }))
                .collect();
            let kept = super::status_mapping(Some(&Value::Array(entries.clone())));
            if kept.as_ref().map_or(0, Vec::len) != entries.len() {
                return Err(at(
                    &format!("{field}.status_mapping"),
                    "maps to a status other than todo, in_progress, in_review, done or cancelled"
                        .into(),
                ));
            }
            m.insert("statusMapping".into(), Value::Array(entries));
        }
        if let Some(w) = &self.default_workflow {
            let value = json!({ "name": w.name, "defaultCronFromMinutes": w.default_cron_from_minutes });
            if super::default_workflow(Some(&value)).is_none() {
                return Err(at(
                    &format!("{field}.default_workflow"),
                    "needs a name and 1 to 1440 minutes".into(),
                ));
            }
            m.insert("defaultWorkflow".into(), value);
        }
        Ok(Value::Object(m))
    }
}

impl RawAction {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("type".into(), json!(self.kind));
        opt(&mut m, "label", self.label.as_ref());
        opt(&mut m, "description", self.description.as_ref());
        if let Some(inputs) = &self.inputs {
            let inputs: Vec<Value> = inputs
                .iter()
                .map(|i| {
                    let mut input = Map::new();
                    input.insert("key".into(), json!(i.key));
                    opt(&mut input, "label", i.label.as_ref());
                    opt(&mut input, "type", i.kind.as_ref());
                    if i.required {
                        input.insert("required".into(), json!(true));
                    }
                    opt(&mut input, "description", i.description.as_ref());
                    if let Some(options) = &i.options {
                        let options: Vec<Value> = options
                            .iter()
                            .map(|o| {
                                let mut option = Map::new();
                                option.insert("value".into(), json!(o.value));
                                opt(&mut option, "label", o.label.as_ref());
                                Value::Object(option)
                            })
                            .collect();
                        input.insert("options".into(), Value::Array(options));
                    }
                    opt(&mut input, "loadOptions", i.load_options.as_ref());
                    Value::Object(input)
                })
                .collect();
            m.insert("inputs".into(), Value::Array(inputs));
        }
        if let Some(outputs) = &self.outputs {
            let outputs: Vec<Value> = outputs
                .iter()
                .map(|o| {
                    let mut output = Map::new();
                    output.insert("key".into(), json!(o.key));
                    opt(&mut output, "type", o.kind.as_ref());
                    opt(&mut output, "description", o.description.as_ref());
                    Value::Object(output)
                })
                .collect();
            m.insert("outputs".into(), Value::Array(outputs));
        }
        Value::Object(m)
    }
}

impl RawEnv {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("name".into(), json!(self.name));
        m.insert("required".into(), json!(self.required));
        m.insert("secret".into(), json!(self.secret));
        opt(&mut m, "description", self.description.as_ref());
        Value::Object(m)
    }
}

impl RawActivation {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        for (key, list) in [
            ("workspaceContains", &self.workspace_contains),
            ("remoteHost", &self.remote_host),
            ("agent", &self.agent),
            ("platform", &self.platform),
        ] {
            if !list.is_empty() {
                m.insert(key.into(), json!(list));
            }
        }
        Value::Object(m)
    }

    fn len(&self) -> usize {
        self.workspace_contains.len() + self.remote_host.len() + self.agent.len() + self.platform.len()
    }
}

/// Every value written must survive the JSON reader's checks.
fn check_activation(
    at: &At<'_>,
    field: &str,
    raw: &RawActivation,
    value: &Value,
) -> Result<(), FileError> {
    let kept = super::activation(Some(value)).unwrap_or_default();
    let count = kept.workspace_contains.len()
        + kept.remote_host.len()
        + kept.agent.len()
        + kept.platform.len();
    if count != raw.len() {
        return Err(at(
            field,
            "names an agent, platform or path this Vorn does not know".into(),
        ));
    }
    Ok(())
}

fn contribution_json(
    id: &str,
    title: Option<&String>,
    description: Option<&String>,
    when: Option<&RawActivation>,
    at: &At<'_>,
    field: &str,
) -> Result<Map<String, Value>, FileError> {
    let mut m = Map::new();
    m.insert("id".into(), json!(id));
    opt(&mut m, "title", title);
    opt(&mut m, "description", description);
    if let Some(when) = when {
        let value = when.to_json();
        check_activation(at, &format!("{field}.when"), when, &value)?;
        m.insert("when".into(), value);
    }
    Ok(m)
}

impl RawContributes {
    fn to_json(&self, at: &At<'_>) -> Result<Value, FileError> {
        let mut out = Map::new();
        let kept = |kind: &str, item: Value| {
            super::contributes(Some(&json!({ kind: [item] })))
        };
        let mut panes = Vec::new();
        for (i, p) in self.panes.iter().enumerate() {
            let field = format!("contributes.panes[{i}]");
            let mut m = contribution_json(
                &p.id,
                p.title.as_ref(),
                p.description.as_ref(),
                p.when.as_ref(),
                at,
                &field,
            )?;
            if let Some(icon) = &p.icon {
                m.insert("icon".into(), icon.to_json());
            }
            opt(&mut m, "web", p.web.as_ref());
            if let Some(command) = &p.command {
                m.insert("command".into(), json!(command));
            }
            let value = Value::Object(m);
            if kept("panes", value.clone()).is_none() {
                return Err(at(
                    &field,
                    "needs an id, and a page under web/ ending in .html or a command".into(),
                ));
            }
            panes.push(value);
        }
        let mut footers = Vec::new();
        for (i, f) in self.footers.iter().enumerate() {
            let field = format!("contributes.footers[{i}]");
            let mut m = contribution_json(
                &f.id,
                f.title.as_ref(),
                f.description.as_ref(),
                f.when.as_ref(),
                at,
                &field,
            )?;
            m.insert("every".into(), json!(f.every));
            let value = Value::Object(m);
            if kept("footers", value.clone()).is_none() {
                return Err(at(
                    &format!("{field}.every"),
                    format!(
                        "is {} s; a footer is read every {} s at most often",
                        f.every,
                        super::MIN_FOOTER_SECONDS
                    ),
                ));
            }
            footers.push(value);
        }
        let mut links = Vec::new();
        for (i, l) in self.link_handlers.iter().enumerate() {
            let field = format!("contributes.link_handlers[{i}]");
            let mut m = contribution_json(
                &l.id,
                l.title.as_ref(),
                l.description.as_ref(),
                l.when.as_ref(),
                at,
                &field,
            )?;
            m.insert("pattern".into(), json!(l.pattern));
            opt(&mut m, "example", l.example.as_ref());
            let value = Value::Object(m);
            if kept("linkHandlers", value.clone()).is_none() {
                return Err(at(
                    &format!("{field}.pattern"),
                    "is not a short regular expression without a nested quantifier".into(),
                ));
            }
            links.push(value);
        }
        for (key, list) in [
            ("panes", panes),
            ("footers", footers),
            ("linkHandlers", links),
        ] {
            if !list.is_empty() {
                out.insert(key.into(), Value::Array(list));
            }
        }
        if out.is_empty() {
            return Err(at("contributes", "contributes nothing".into()));
        }
        Ok(Value::Object(out))
    }
}

// ---- the other way: a manifest written out ----

impl From<&Manifest> for Raw {
    fn from(m: &Manifest) -> Raw {
        let icon = |i: &super::Icon| RawIcon {
            view_box: Some(i.view_box.clone()),
            paths: i.paths.clone(),
        };
        let when = |a: &super::Activation| RawActivation {
            workspace_contains: a.workspace_contains.clone(),
            remote_host: a.remote_host.clone(),
            agent: a.agent.clone(),
            platform: a.platform.clone(),
        };
        let is_connector = !m.triggers.is_empty() || !m.actions.is_empty() || m.auth.is_some();
        let connector = is_connector.then(|| RawConnector {
            auth: m.auth.as_ref().map(|a| RawAuth {
                rung: a.rung.clone(),
                probe: a.probe.as_ref().map(|p| RawProbe {
                    command: p.command.clone(),
                    args: p.args.clone(),
                }),
                borrow: a.borrow.as_ref().map(|b| RawBorrow {
                    env: b.env.clone(),
                    token_args: b.token_args.clone(),
                    token_env: b.token_env.clone(),
                }),
                keys: a.keys.clone(),
                browser: a.browser.as_ref().map(|b| RawBrowser {
                    sign_in_url: b.sign_in_url.clone(),
                    origins: b.origins.clone(),
                    check: RawCheck {
                        url: b.check.url.clone(),
                        identity: b.check.identity.clone(),
                        headers: b.check.headers.as_ref().map(|h| {
                            h.iter()
                                .map(|(k, v)| (k.clone(), crate::js::to_string(v)))
                                .collect()
                        }),
                    },
                }),
            }),
            triggers: m
                .triggers
                .iter()
                .map(|t| RawTrigger {
                    kind: t.kind.clone(),
                    label: (t.label != t.kind).then(|| t.label.clone()),
                    description: t.description.clone(),
                    status_mapping: t.status_mapping.as_ref().map(|s| {
                        s.iter()
                            .map(|s| RawStatusMapping {
                                upstream: s.upstream.clone(),
                                suggested_local: s.suggested_local.clone(),
                            })
                            .collect()
                    }),
                    default_workflow: t.default_workflow.as_ref().map(|w| RawDefaultWorkflow {
                        name: w.name.clone(),
                        default_cron_from_minutes: w.default_cron_from_minutes,
                    }),
                })
                .collect(),
            actions: m
                .actions
                .iter()
                .map(|a| RawAction {
                    kind: a.kind.clone(),
                    label: (a.label != a.kind).then(|| a.label.clone()),
                    description: a.description.clone(),
                    inputs: a.inputs.as_ref().map(|inputs| {
                        inputs
                            .iter()
                            .map(|i| RawInput {
                                key: i.key.clone(),
                                label: (i.label != i.key).then(|| i.label.clone()),
                                kind: (i.kind != "string").then(|| i.kind.clone()),
                                required: i.required,
                                description: i.description.clone(),
                                options: i.options.as_ref().map(|o| {
                                    o.iter()
                                        .map(|o| RawOption {
                                            value: o.value.clone(),
                                            label: o.label.clone(),
                                        })
                                        .collect()
                                }),
                                load_options: i.load_options.clone(),
                            })
                            .collect()
                    }),
                    outputs: a.outputs.as_ref().map(|outputs| {
                        outputs
                            .iter()
                            .map(|o| RawOutput {
                                key: o.key.clone(),
                                kind: o.kind.clone(),
                                description: o.description.clone(),
                            })
                            .collect()
                    }),
                })
                .collect(),
            env: m
                .env
                .iter()
                .map(|e| RawEnv {
                    name: e.name.clone(),
                    required: e.required,
                    secret: e.secret,
                    description: e.description.clone(),
                })
                .collect(),
        });
        let contributes = m.contributes.as_ref().map(|c| RawContributes {
            panes: c
                .panes()
                .iter()
                .map(|p| {
                    let (web, command) = match &p.draws {
                        super::PaneDraws::Web(w) => (Some(w.clone()), None),
                        super::PaneDraws::Command(c) => (None, Some(c.clone())),
                    };
                    RawPane {
                        id: p.base.id.clone(),
                        title: (p.base.title != p.base.id).then(|| p.base.title.clone()),
                        description: p.base.description.clone(),
                        when: p.base.when.as_ref().map(when),
                        icon: p.icon.as_ref().map(icon),
                        web,
                        command,
                    }
                })
                .collect(),
            footers: c
                .footers()
                .iter()
                .map(|f| RawFooter {
                    id: f.base.id.clone(),
                    title: (f.base.title != f.base.id).then(|| f.base.title.clone()),
                    description: f.base.description.clone(),
                    when: f.base.when.as_ref().map(when),
                    every: f.every,
                })
                .collect(),
            link_handlers: c
                .link_handlers()
                .iter()
                .map(|l| RawLink {
                    id: l.base.id.clone(),
                    title: (l.base.title != l.base.id).then(|| l.base.title.clone()),
                    description: l.base.description.clone(),
                    when: l.base.when.as_ref().map(when),
                    pattern: l.pattern.clone(),
                    example: l.example.clone(),
                })
                .collect(),
        });
        let uses = if m.declares.uses.is_empty() {
            m.permissions
                .iter()
                .flatten()
                .map(|p| p.name().to_owned())
                .collect()
        } else {
            m.declares.uses.clone()
        };
        Raw {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            api: m.api.clone().unwrap_or_default(),
            vorn: m.vorn.clone(),
            publisher: m.publisher.clone(),
            description: m.description.clone(),
            icon: m.icon.as_ref().map(icon),
            build: m.build.clone().map(|command| RawCommand { command }),
            service: m.service.as_ref().map(|s| RawService {
                command: s.command.clone(),
                events: s.events.clone(),
            }),
            actions: m
                .runs
                .iter()
                .map(|r| {
                    (
                        r.id.clone(),
                        RawRun {
                            title: (r.title != r.id).then(|| r.title.clone()),
                            command: r.command.clone(),
                        },
                    )
                })
                .collect(),
            checks: m.checks.clone().map(|command| RawCommand { command }),
            declares: RawDeclares {
                network: m.declares.network.clone(),
                uses,
            },
            credential: m.credential.as_ref().map(|c| RawCredential {
                field: c.field.clone(),
                header: c.header.clone(),
            }),
            connector,
            activates: m.activates.as_ref().map(when),
            contributes,
        }
    }
}

// ---- where each field sits ----

/// The line every key of a TOML file sits on, by the dotted path the checks
/// report (`contributes.panes[1].web`).
#[derive(Debug, Default)]
struct Lines {
    at: HashMap<String, usize>,
}

impl Lines {
    fn parse(text: &str) -> Result<Lines, FileError> {
        let table = DeTable::parse(text).map_err(|e| FileError {
            line: e.span().map_or(1, |s| line_of(text, s.start)),
            field: String::new(),
            message: e.message().trim_end().to_owned(),
        })?;
        let mut lines = Lines::default();
        lines.table(text, table.get_ref(), "");
        Ok(lines)
    }

    /// The line of `path`, or of the nearest enclosing path that has one.
    fn line(&self, path: &str) -> usize {
        let mut p = path;
        loop {
            if let Some(l) = self.at.get(p) {
                return *l;
            }
            match p.rfind(['.', '[']) {
                Some(cut) => p = &p[..cut],
                None => return 1,
            }
        }
    }

    fn table(
        &mut self,
        text: &str,
        t: &toml::map::Map<Spanned<std::borrow::Cow<'_, str>>, Spanned<DeValue<'_>>>,
        path: &str,
    ) {
        for (k, v) in t {
            let p = if path.is_empty() {
                k.get_ref().to_string()
            } else {
                format!("{path}.{}", k.get_ref())
            };
            self.at.insert(p.clone(), line_of(text, k.span().start));
            self.value(text, v, &p);
        }
    }

    fn value(&mut self, text: &str, v: &Spanned<DeValue<'_>>, path: &str) {
        match v.get_ref() {
            DeValue::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    let p = format!("{path}[{i}]");
                    self.at.insert(p.clone(), line_of(text, item.span().start));
                    self.value(text, item, &p);
                }
            }
            DeValue::Table(t) => self.table(text, t, path),
            _ => {}
        }
    }
}

/// The 1-based line of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE: &str = r#"id = "example.pr-checks"
name = "PR checks"
version = "0.1.0"
api = "vorn:extension@0.9"
vorn = ">=0.9"

[service]
command = ["node", "dist/main.js"]
events = ["session.exited"]

[actions.rerun]
title = "Re-run failed checks"
command = ["bash", "rerun.sh"]

[declares]
network = ["api.example.com"]
uses = ["sessions.read", "auth.borrow:gh"]

[credential]
field = "token"
header = "Authorization: Bearer {token}"
"#;

    #[test]
    fn reads_a_program_extension() {
        let m = parse(SERVICE).unwrap();
        assert!(m.speaks_api());
        assert_eq!(m.protocol, None);
        assert_eq!(m.run("rerun").unwrap().command, ["bash", "rerun.sh"]);
        assert_eq!(m.service.as_ref().unwrap().events, ["session.exited"]);
        assert_eq!(
            m.permissions,
            Some(vec![Permission::SessionsRead, Permission::AuthBorrow])
        );
        assert_eq!(m.declares.uses, ["sessions.read", "auth.borrow:gh"]);
        assert_eq!(m.kind, super::super::Kind::Connector);
        // Written out and read back, it is the same manifest.
        assert_eq!(parse(&to_toml(&m)).unwrap(), m);
    }

    #[test]
    fn names_the_line_and_field_of_a_bad_value() {
        let err = parse(&SERVICE.replace("title =", "titel =")).unwrap_err();
        assert_eq!(err.line, 12);
        assert!(err.message.contains("unknown field `titel`"), "{err}");
        assert!(err.to_string().starts_with("vorn-extension.toml:12: "));

        let err = parse(&SERVICE.replace("sessions.read", "session.read")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "vorn-extension.toml:17: declares.uses[0]: unknown permission `session.read`"
        );
        let err = parse(&SERVICE.replace("api.example.com", "API.example.com/x")).unwrap_err();
        assert_eq!(err.line, 16);
        assert_eq!(err.field, "declares.network[0]");
        let err = parse(&SERVICE.replace("@0.9\"", "@2.0\"")).unwrap_err();
        assert_eq!((err.line, err.field.as_str()), (4, "api"));
        let err = parse(&SERVICE.replace("{token}", "{tok}")).unwrap_err();
        assert_eq!((err.line, err.field.as_str()), (21, "credential.header"));
        let err = parse(&SERVICE.replace("[\"bash\", \"rerun.sh\"]", "[]")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "vorn-extension.toml:13: actions.rerun.command: is empty; it names the program to run"
        );
        let err = parse(&SERVICE.replace("session.exited", "exited")).unwrap_err();
        assert_eq!((err.line, err.field.as_str()), (9, "service.events[0]"));
        let err = parse("id = \"x\"\nname = = 1\n").unwrap_err();
        assert_eq!(err.line, 2);
    }

    #[test]
    fn needs_something_to_run_or_show() {
        let bare = "id = \"x\"\nname = \"X\"\nversion = \"1.0.0\"\napi = \"vorn:extension@0.9\"\n";
        assert_eq!(
            parse(bare).unwrap_err().message,
            "declares no service, action, connector or contribution"
        );
    }

    #[test]
    fn holds_legacy_parts_to_the_connector_protocol() {
        let text = r#"id = "x"
name = "X"
version = "1.0.0"
api = "vorn:extension@0.9"

[[contributes.footers]]
id = "f"
every = 5
"#;
        let err = parse(text).unwrap_err();
        assert_eq!((err.line, err.field.as_str()), (6, "contributes"));
        let ok = parse(&text.replace("vorn:extension@0.9", "vorn:connector@1")).unwrap();
        assert_eq!(ok.kind, super::super::Kind::Extension);
        assert_eq!(ok.protocol, Some(1));
        let err = parse(
            &text
                .replace("vorn:extension@0.9", "vorn:connector@1")
                .replace("every = 5", "every = 1"),
        )
        .unwrap_err();
        assert_eq!(err.field, "contributes.footers[0].every");
        assert_eq!(err.line, 8);
    }

    #[test]
    fn converts_a_pack_and_back() {
        let pack = json!({
            "id": "jira", "name": "Jira", "version": "1.2.0", "protocol": 1,
            "auth": { "rung": "cli", "probe": { "command": "acli", "args": ["auth", "status"] } },
            "triggers": [{ "type": "issue", "label": "Issues", "statusMapping": [{ "upstream": "Open", "suggestedLocal": "todo" }],
                           "setup": { "env": [{ "name": "JIRA_URL", "required": true }] } }],
            "actions": [{ "type": "comment", "inputs": [{ "key": "body", "type": "text", "required": true }] }]
        });
        let m = from_pack_json(pack.as_object().unwrap()).unwrap();
        let text = to_toml(&m);
        assert!(text.contains("api = \"vorn:connector@1\""), "{text}");
        assert!(text.contains("\"index.js\""), "{text}");
        assert_eq!(parse(&text).unwrap(), m);
    }

    #[test]
    fn reads_ranges_by_their_form() {
        for ok in [">=0.9", ">=0.9.0, <2", "^1.2.3", "=0.9.0-beta.1", "1.*"] {
            assert!(is_range(ok), "{ok}");
        }
        for bad in ["", ">=", "=>0.9", "latest", ">=a.9"] {
            assert!(!is_range(bad), "{bad}");
        }
    }
}

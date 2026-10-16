//! The agents vornd runs, from their adapters: the ones Vorn ships
//! ([`vorn_adapters`]), each behind its switch, and any installed package
//! with an `[agent]` section.
//!
//! Every reader takes a snapshot ([`Adapters::agents`]) and holds it for the
//! call: a switch that moves meanwhile changes the next call, never one in
//! flight. With every adapter off the snapshot is empty and the launcher
//! opens a plain shell.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, PoisonError, RwLock};

use serde_json::{json, Map, Value};
use tracing::warn;
use vorn_agents::{Agent, Agents};

/// A package an adapter is read from.
struct Package<'a> {
    name: &'a str,
    manifest: &'a str,
    on: bool,
    file: Box<dyn Fn(&str) -> Option<&'a [u8]> + 'a>,
}

/// One adapter vornd knows, on or off.
#[derive(Clone, Debug)]
struct Known {
    agent: Agent,
    /// The extension it comes in.
    extension: String,
    /// Its mark, as SVG text.
    icon: Option<String>,
    builtin: bool,
    on: bool,
}

#[derive(Debug, Default)]
struct State {
    shipped: Vec<Known>,
    installed: Vec<Known>,
    current: Arc<Agents>,
}

impl State {
    fn rebuild(&mut self) {
        let on = self.shipped.iter().chain(&self.installed).filter(|k| k.on);
        self.current = Arc::new(Agents::new(on.map(|k| k.agent.clone())));
    }
}

/// The adapters, and which are on.
#[derive(Debug)]
pub struct Adapters {
    state: RwLock<State>,
    /// Bumped whenever which agents are on changes.
    changed: tokio::sync::watch::Sender<u64>,
}

/// An installed package's adapter, as the extension host read it.
#[derive(Clone, Debug)]
pub struct Installed {
    pub extension: String,
    pub agent: Agent,
    pub icon: Option<String>,
}

impl Adapters {
    /// The adapters this build runs with until it is told otherwise: the
    /// shipped ones, or under this crate's tests the test adapters.
    pub fn standard() -> Arc<Adapters> {
        #[cfg(test)]
        return testing::adapters();
        #[cfg(not(test))]
        Adapters::shipped()
    }

    /// The adapters Vorn ships, each as its switch defaults.
    pub fn shipped() -> Arc<Adapters> {
        Adapters::from_packages(vorn_adapters::shipped().iter().map(|s| Package {
            name: s.name,
            manifest: s.manifest,
            on: s.on_by_default,
            file: Box::new(|p: &str| s.file(p)),
        }))
    }

    fn from_packages<'a>(packages: impl Iterator<Item = Package<'a>>) -> Arc<Adapters> {
        let shipped: Vec<Known> = packages
            .filter_map(|s| {
                let agent = match vorn_agents::from_manifest_text(s.manifest) {
                    Ok(Some(agent)) => agent,
                    Ok(None) => return None,
                    Err(err) => {
                        warn!(adapter = s.name, %err, "a shipped adapter does not read");
                        return None;
                    }
                };
                let extension = extension_id(s.manifest).unwrap_or_else(|| s.name.to_owned());
                let icon = icon_path(&agent)
                    .and_then(|p| (s.file)(&p))
                    .and_then(|b| String::from_utf8(b.to_vec()).ok());
                Some(Known {
                    agent,
                    extension,
                    icon,
                    builtin: true,
                    on: s.on,
                })
            })
            .collect();
        let mut state = State {
            shipped,
            ..State::default()
        };
        state.rebuild();
        Arc::new(Adapters {
            state: RwLock::new(state),
            changed: tokio::sync::watch::Sender::new(0),
        })
    }

    /// The agents that are on, in the launcher's order.
    pub fn agents(&self) -> Arc<Agents> {
        Arc::clone(&self.read().current)
    }

    /// The agent `id` names among those on.
    pub fn get(&self, id: &str) -> Option<Agent> {
        self.read().current.get(id).cloned()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Changes the state and tells whoever follows which agents are on.
    fn change(&self, f: impl FnOnce(&mut State)) {
        {
            let mut state = self.write();
            f(&mut state);
            state.rebuild();
        }
        self.changed.send_modify(|n| *n += 1);
    }

    /// Wakes whenever which agents are on changes.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Switches the shipped adapter of extension `extension` on or off.
    pub fn set_shipped(&self, extension: &str, on: bool) {
        self.change(|state| {
            for k in state
                .shipped
                .iter_mut()
                .filter(|k| k.extension == extension)
            {
                k.on = on;
            }
        });
    }

    /// Whether `extension` is one of the shipped adapters.
    pub fn ships(&self, extension: &str) -> bool {
        self.read().shipped.iter().any(|k| k.extension == extension)
    }

    /// Reads the shipped adapters' switches from the built-ins' file, as the
    /// extension host keeps them, before the host has started.
    pub fn load_switches(&self, file: &Path) {
        let saved: BTreeMap<String, bool> = std::fs::read(file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        self.change(|state| {
            for k in &mut state.shipped {
                if let Some(on) = saved.get(&k.extension) {
                    k.on = *on;
                }
            }
        });
    }

    /// The installed packages' adapters, after the shipped ones; a later one
    /// claiming an id already taken is left out.
    pub fn set_installed(&self, installed: Vec<Installed>) {
        let installed: Vec<Known> = installed
            .into_iter()
            .map(|i| Known {
                agent: i.agent,
                extension: i.extension,
                icon: i.icon,
                builtin: false,
                on: true,
            })
            .collect();
        self.change(|state| state.installed = installed);
    }

    /// `agent:list`: each agent that is on, as the launcher, the pickers and
    /// Settings draw it, in their order.
    pub fn describe(&self) -> Value {
        let state = self.read();
        let list = state
            .current
            .iter()
            .filter_map(|agent| {
                let known = state
                    .shipped
                    .iter()
                    .chain(&state.installed)
                    .find(|k| k.on && k.agent == *agent)?;
                Some(describe(known))
            })
            .collect();
        Value::Array(list)
    }
}

/// What a client draws of one agent.
fn describe(k: &Known) -> Value {
    let spec = k.agent.spec();
    let mut out = Map::new();
    let mut put = |key: &str, v: Value| {
        if !v.is_null() {
            out.insert(key.to_owned(), v);
        }
    };
    put("id", json!(spec.id));
    put("name", json!(spec.name));
    put("label", json!(spec.label()));
    put("shortName", json!(spec.short_name()));
    put("description", json!(spec.description));
    put("install", json!(spec.install));
    put("url", json!(spec.url));
    put("command", json!(spec.launch.command));
    put("args", json!(spec.launch.args));
    put("headlessArgs", json!(spec.launch.headless_args));
    put("color", json!(spec.color));
    put("tint", json!(spec.tint));
    put("mark", json!(spec.mark));
    put("icon", json!(k.icon));
    put("models", json!(k.agent.selects_models()));
    put(
        "groupModels",
        json!(spec.models.as_ref().is_some_and(|m| m.group)),
    );
    put("headless", json!(spec.headless.is_some()));
    put("resumes", json!(k.agent.resumes_exactly()));
    if let Some(mcp) = &spec.mcp {
        put(
            "mcp",
            json!({ "commands": mcp.commands, "inAgent": mcp.in_agent, "note": mcp.note }),
        );
    }
    if let Some(env) = &spec.env {
        put(
            "envMarkers",
            json!({ "names": env.strip, "prefixes": env.strip_prefixes }),
        );
    }
    put("extension", json!(k.extension));
    put("builtin", json!(k.builtin));
    Value::Object(out)
}

/// The `id` at the top of a manifest's text.
fn extension_id(manifest: &str) -> Option<String> {
    let table: toml::Table = toml::from_str(manifest).ok()?;
    table.get("id")?.as_str().map(str::to_owned)
}

/// Where in its package an adapter's mark is.
pub fn icon_path(agent: &Agent) -> Option<String> {
    agent.spec().icon.clone()
}

#[cfg(test)]
pub(crate) mod testing {
    //! Adapters for this crate's tests, in the public format, named for no
    //! agent CLI: `acme` resumes and pins conversations, lists its models and
    //! installs settings hooks; `beta` resumes by a subcommand and captures
    //! its conversation from a database; `gamma` only launches.

    use super::*;

    pub const ACME: &str = r##"
id = "test.acme"
name = "Acme"
version = "1.0.0"
api = "vorn:extension@0.9"
[agent]
id = "acme"
name = "Acme Code"
label = "Acme"
icon = "icon.svg"
mark = "*"
color = "#d97757"
[agent.launch]
command = "acme"
headless_args = ["--yes"]
[agent.resume]
args = ["--resume", "{id}"]
pin = ["--session-id", "{id}"]
selectors = { with_value = ["--resume", "--session-id"] }
[agent.headless]
prompt = ["-p"]
no_prompt = ["-p", ""]
resume = ["--resume", "{id}"]
[agent.models]
[agent.models.list]
args = ["models"]
lines = '^\S+$'
[agent.history]
activity = "entry"
[agent.history.jsonl]
file = "~/.acme/history.jsonl"
session = "sessionId"
project = "project"
time = "timestamp"
title = "display"
[agent.hooks.settings]
file = "~/.acme/settings.json"
events = ["PreToolUse", "PostToolUse", "Stop", "PermissionRequest", "SessionStart", "SessionEnd"]
own_status = { since = "2.0.0", events = ["SessionStart", "PermissionRequest"] }
[agent.env]
strip = ["ACMECODE"]
strip_prefixes = ["ACME_CODE_"]
[[agent.status]]
status = "idle"
any = ['^> $']
"##;

    pub const BETA: &str = r##"
id = "test.beta"
name = "Beta"
version = "1.0.0"
api = "vorn:extension@0.9"
[agent]
id = "beta"
name = "Beta CLI"
[agent.launch]
command = "beta"
[agent.resume]
args = ["resume", "{id}"]
at = "start"
selectors = { subcommand = "resume" }
capture = { db = "~/.beta/state.sqlite", query = "SELECT id FROM threads WHERE cwd = ?1 LIMIT 1" }
[agent.headless]
prompt = ["exec"]
no_prompt = ["exec", ""]
resume_tail = ["exec", "resume", "{id}", "-"]
[agent.models]
"##;

    pub const GAMMA: &str = r##"
id = "test.gamma"
name = "Gamma"
version = "1.0.0"
api = "vorn:extension@0.9"
[agent]
id = "gamma"
name = "Gamma CLI"
[agent.launch]
command = "gamma"
[agent.headless]
no_prompt = ["-p", ""]
"##;

    const ICON: &[u8] =
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><path fill=\"#d97757\" d=\"M0 0h1v1z\"/></svg>";

    /// The three test adapters, all on.
    pub fn adapters() -> Arc<Adapters> {
        Adapters::from_packages(
            [("acme", ACME), ("beta", BETA), ("gamma", GAMMA)]
                .into_iter()
                .map(|(name, manifest)| Package {
                    name,
                    manifest,
                    on: true,
                    file: Box::new(|p: &str| (p == "icon.svg").then_some(ICON)),
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_adapters_all_read() {
        let shipped = Adapters::shipped();
        assert_eq!(shipped.read().shipped.len(), vorn_adapters::shipped().len());
    }

    #[test]
    fn switches_adapters_and_keeps_the_launcher_s_order() {
        let adapters = testing::adapters();
        let on: Vec<String> = adapters
            .agents()
            .iter()
            .map(|a| a.id().to_owned())
            .collect();
        assert!(!on.is_empty());
        let first = adapters.agents().first().cloned().unwrap();
        let listed = adapters.describe();
        assert_eq!(listed[0]["id"], first.id());
        assert!(listed[0]["icon"]
            .as_str()
            .is_some_and(|s| s.contains("<svg")));
        let ext = listed[0]["extension"].as_str().unwrap().to_owned();
        adapters.set_shipped(&ext, false);
        assert!(adapters.get(first.id()).is_none());
        assert_eq!(adapters.agents().len(), on.len() - 1);

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("builtins.json");
        let off: BTreeMap<String, bool> = [testing::ACME, testing::BETA, testing::GAMMA]
            .iter()
            .filter_map(|m| extension_id(m))
            .map(|id| (id, false))
            .collect();
        std::fs::write(&file, serde_json::to_vec(&off).unwrap()).unwrap();
        adapters.load_switches(&file);
        assert!(adapters.agents().is_empty());
        assert_eq!(adapters.describe(), json!([]));
    }
}

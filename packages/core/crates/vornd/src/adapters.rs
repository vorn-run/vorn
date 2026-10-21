//! The agents vornd runs, from their adapters: every installed package
//! with an `[agent]` section, Vorn's own among them as ordinary installs.
//!
//! Every reader takes a snapshot ([`Adapters::agents`]) and holds it for the
//! call: an install or removal meanwhile changes the next call, never one in
//! flight. With no adapter installed the snapshot is empty and the launcher
//! opens a plain shell.

use std::path::Path;
use std::sync::{Arc, PoisonError, RwLock};

use serde_json::{json, Map, Value};
use tracing::warn;
use vorn_agents::{Agent, Agents};
use vorn_extensions::pack::PackStore;

#[derive(Debug, Default)]
struct State {
    installed: Vec<Installed>,
    current: Arc<Agents>,
}

/// The installed adapters, in the launcher's order.
#[derive(Debug)]
pub struct Adapters {
    state: RwLock<State>,
    /// Bumped whenever which agents are installed changes.
    changed: tokio::sync::watch::Sender<u64>,
}

/// An installed package's adapter.
#[derive(Clone, Debug)]
pub struct Installed {
    pub extension: String,
    pub agent: Agent,
    /// Its mark, as SVG text.
    pub icon: Option<String>,
}

impl Adapters {
    /// The adapters this build runs with until the packages are read: none,
    /// or under this crate's tests the test adapters.
    pub fn standard() -> Arc<Adapters> {
        #[cfg(test)]
        return testing::adapters();
        #[cfg(not(test))]
        Adapters::with(Vec::new())
    }

    /// The adapters the packages under `packages` install.
    pub fn installed_in(packages: &Path) -> Arc<Adapters> {
        Adapters::with(read_installed(&PackStore::new(packages)))
    }

    fn with(installed: Vec<Installed>) -> Arc<Adapters> {
        let adapters = Arc::new(Adapters {
            state: RwLock::new(State::default()),
            changed: tokio::sync::watch::Sender::new(0),
        });
        adapters.set_installed(installed);
        adapters
    }

    /// The agents installed, in the launcher's order.
    pub fn agents(&self) -> Arc<Agents> {
        Arc::clone(&self.read().current)
    }

    /// The agent `id` names among those installed.
    pub fn get(&self, id: &str) -> Option<Agent> {
        self.read().current.get(id).cloned()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes whenever which agents are installed changes.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// The extension whose adapter runs agent `id`.
    pub fn extension_of(&self, id: &str) -> Option<String> {
        self.read()
            .installed
            .iter()
            .find(|k| k.agent.id() == id)
            .map(|k| k.extension.clone())
    }

    /// Reads the adapters installed under `packages`, as the extension host
    /// keeps them, before the host has started.
    pub fn load_installed(&self, packages: &Path) {
        let installed = read_installed(&PackStore::new(packages));
        // This crate's tests run their test adapters beside whatever they install.
        #[cfg(test)]
        let installed = testing::installed().into_iter().chain(installed).collect();
        self.set_installed(installed);
    }

    /// The installed packages' adapters; a later one claiming an id already
    /// taken is left out.
    pub fn set_installed(&self, installed: Vec<Installed>) {
        {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            state.current = Arc::new(Agents::new(installed.iter().map(|k| k.agent.clone())));
            state.installed = installed;
        }
        self.changed.send_modify(|n| *n += 1);
    }

    /// `agent:list`: each installed agent, as the launcher, the pickers and
    /// Settings draw it, in their order.
    pub fn describe(&self) -> Value {
        let state = self.read();
        let list = state
            .current
            .iter()
            .filter_map(|agent| {
                let known = state.installed.iter().find(|k| k.agent == *agent)?;
                Some(describe(known))
            })
            .collect();
        Value::Array(list)
    }
}

/// Every adapter the packages in `store` install, in the store's order.
pub fn read_installed(store: &PackStore) -> Vec<Installed> {
    let mut installed = Vec::new();
    for id in store.ids() {
        let Ok(pack) = store.inspect_unmeasured(&id) else {
            continue;
        };
        let Some(section) = pack.manifest.agent.clone() else {
            continue;
        };
        let agent = match vorn_agents::AgentSpec::from_toml(section).and_then(Agent::new) {
            Ok(agent) => agent,
            Err(err) => {
                warn!("[extensions] the agent adapter in {id} does not read: {err}");
                continue;
            }
        };
        let icon = icon_path(&agent)
            .filter(|p| !p.contains("..") && !Path::new(p).is_absolute())
            .and_then(|p| std::fs::read_to_string(pack.path.join(p)).ok());
        installed.push(Installed {
            extension: id,
            agent,
            icon,
        });
    }
    installed
}

/// What a client draws of one agent.
fn describe(k: &Installed) -> Value {
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
    Value::Object(out)
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
api = "vorn:extension@1.0"
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
api = "vorn:extension@1.0"
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
api = "vorn:extension@1.0"
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

    /// The three test adapters, installed.
    pub fn adapters() -> Arc<Adapters> {
        Adapters::with(installed())
    }

    /// The three test adapters.
    pub fn installed() -> Vec<Installed> {
        [
            ("test.acme", ACME),
            ("test.beta", BETA),
            ("test.gamma", GAMMA),
        ]
        .into_iter()
        .filter_map(|(extension, manifest)| {
            let agent = vorn_agents::from_manifest_text(manifest).ok()??;
            let icon = icon_path(&agent)
                .is_some()
                .then(|| String::from_utf8_lossy(ICON).into_owned());
            Some(Installed {
                extension: extension.to_owned(),
                agent,
                icon,
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_what_is_installed_and_runs_none_once_every_adapter_is_gone() {
        let adapters = testing::adapters();
        let on = adapters.agents().len();
        assert!(on > 0);
        let listed = adapters.describe();
        let first = listed[0]["id"].as_str().unwrap().to_owned();
        assert!(listed[0]["icon"]
            .as_str()
            .is_some_and(|s| s.contains("<svg")));
        assert_eq!(
            adapters.extension_of(&first).as_deref(),
            listed[0]["extension"].as_str()
        );

        let dir = tempfile::tempdir().unwrap();
        adapters.set_installed(read_installed(&PackStore::new(dir.path())));
        assert!(adapters.agents().is_empty());
        assert!(adapters.get(&first).is_none());
        assert_eq!(adapters.describe(), json!([]));
    }

    #[test]
    fn reads_an_installed_package_s_adapter_and_its_mark() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("connectors/test.acme/1.0.0");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("vorn-extension.toml"), testing::ACME).unwrap();
        std::fs::write(pack.join("icon.svg"), "<svg/>").unwrap();
        std::fs::write(
            dir.path().join("connectors/test.acme/current.json"),
            r#"{"version":"1.0.0"}"#,
        )
        .unwrap();
        let adapters = Adapters::installed_in(&dir.path().join("connectors"));
        let listed = adapters.describe();
        assert_eq!(listed[0]["id"], "acme");
        assert_eq!(listed[0]["extension"], "test.acme");
        assert_eq!(listed[0]["icon"], "<svg/>");
    }
}

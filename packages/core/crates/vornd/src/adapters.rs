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
use vorn_extensions::pack::{InstalledPack, PackStore};

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

/// Every adapter the packages in `store` install, Vorn's own first, then by
/// id. An agent runs one adapter: Vorn's own keeps its agent id, then the one
/// that claimed it first, by `installedAt`. A linked folder whose manifest
/// changed since it was linked claimed its agent last.
pub fn read_installed(store: &PackStore) -> Vec<Installed> {
    let mut read: Vec<(bool, (bool, u64), InstalledPack, Agent)> = store
        .ids()
        .into_iter()
        .filter_map(|id| {
            let pack = store.inspect_unmeasured(&id).ok()?;
            let section = pack.manifest.agent.clone()?;
            let agent = vorn_agents::AgentSpec::from_toml(section)
                .and_then(Agent::new)
                .map_err(|err| warn!("[extensions] the agent adapter in {id} does not read: {err}"))
                .ok()?;
            let ours = id.starts_with(vorn_extensions::pack::SHIPPED_NAMESPACE);
            let edited = pack.linked
                && (pack.linked_agent.as_deref() != Some(&agent.spec().id)
                    || pack.linked_manifest != manifest_digest(&pack.path));
            let claimed = (edited, pack.installed_at.as_u64().unwrap_or(u64::MAX));
            Some((!ours, claimed, pack, agent))
        })
        .collect();
    // Claim order decides who keeps an agent; the store's id order settles a tie.
    read.sort_by_key(|(theirs, claimed, ..)| (*theirs, *claimed));
    let mut installed: Vec<Installed> = Vec::new();
    for (_, _, pack, agent) in read {
        let agent_id = &agent.spec().id;
        if let Some(first) = installed.iter().find(|k| k.agent.spec().id == *agent_id) {
            warn!(
                "[extensions] {} adds the agent \"{agent_id}\", which {} already runs; skipped",
                pack.id, first.extension
            );
            continue;
        }
        let icon = icon_path(&agent)
            .filter(|p| !p.contains("..") && !Path::new(p).is_absolute())
            .and_then(|p| std::fs::read_to_string(pack.path.join(p)).ok());
        installed.push(Installed {
            extension: pack.id,
            agent,
            icon,
        });
    }
    installed.sort_by(|a, b| {
        let key = |k: &Installed| {
            !k.extension
                .starts_with(vorn_extensions::pack::SHIPPED_NAMESPACE)
        };
        key(a)
            .cmp(&key(b))
            .then_with(|| a.extension.cmp(&b.extension))
    });
    installed
}

/// What a client draws of one agent.
/// The sha256 of the manifest in folder `dir`, as a link records it.
fn manifest_digest(dir: &Path) -> Option<String> {
    let text = std::fs::read(dir.join(vorn_extensions::manifest::file::FILE)).ok()?;
    Some(vorn_extensions::install::sha256_hex(&text))
}

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

    #[test]
    fn vorns_own_adapter_keeps_its_agent_over_one_installed_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        // Placed by hand, as a pack installed before installs refused a second claim would be.
        for id in ["acme.x", "vorn.agents.acme"] {
            let pack = dir.path().join(id).join("1.0.0");
            std::fs::create_dir_all(&pack).unwrap();
            let manifest = testing::ACME.replace("id = \"test.acme\"", &format!("id = \"{id}\""));
            std::fs::write(pack.join("vorn-extension.toml"), manifest).unwrap();
            std::fs::write(
                dir.path().join(id).join("current.json"),
                r#"{"version":"1.0.0"}"#,
            )
            .unwrap();
        }
        let adapters = Adapters::installed_in(dir.path());
        assert_eq!(adapters.agents().len(), 1);
        assert_eq!(
            adapters.extension_of("acme").as_deref(),
            Some("vorn.agents.acme")
        );
        assert_eq!(adapters.describe()[0]["extension"], "vorn.agents.acme");
    }

    /// Writes an adapter for `agent` as pack `id`, placed or linked, claimed at `at`.
    fn adapter(root: &Path, id: &str, agent: &str, at: u64, linked: Option<&Path>) {
        let manifest = testing::ACME
            .replace("id = \"test.acme\"", &format!("id = \"{id}\""))
            .replace("id = \"acme\"", &format!("id = \"{agent}\""));
        let home = root.join(id);
        let current = match linked {
            Some(folder) => {
                std::fs::write(folder.join("vorn-extension.toml"), &manifest).unwrap();
                let digest = vorn_extensions::install::sha256_hex(manifest.as_bytes());
                serde_json::json!({
                    "linked": folder,
                    "installedAt": at,
                    "linkedAgent": agent,
                    "linkedManifest": digest,
                })
            }
            None => {
                std::fs::create_dir_all(home.join("1.0.0")).unwrap();
                std::fs::write(home.join("1.0.0/vorn-extension.toml"), manifest).unwrap();
                serde_json::json!({ "version": "1.0.0", "installedAt": at })
            }
        };
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("current.json"), current.to_string()).unwrap();
    }

    fn extensions_in(root: &Path) -> Vec<String> {
        read_installed(&PackStore::new(root))
            .into_iter()
            .map(|k| k.extension)
            .collect()
    }

    #[test]
    fn the_adapter_that_claimed_an_agent_first_keeps_it_installed_or_linked() {
        let dir = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        // Placed by hand, as packs installed before installs refused a second claim would be.
        adapter(dir.path(), "acme.a", "acme", 30, None);
        adapter(dir.path(), "acme.b", "acme", 20, None);
        adapter(dir.path(), "acme.0linked", "acme", 40, Some(folder.path()));
        adapter(dir.path(), "acme.0zeta", "zeta", 50, None);
        assert_eq!(
            extensions_in(dir.path()),
            ["acme.0zeta", "acme.b"],
            "the earliest claim keeps it, and the list stays in id order"
        );

        adapter(dir.path(), "acme.0linked", "acme", 10, Some(folder.path()));
        assert_eq!(
            extensions_in(dir.path()),
            ["acme.0linked", "acme.0zeta"],
            "an earlier link keeps it"
        );
    }

    #[test]
    fn a_link_edited_to_adapt_a_running_agent_does_not_take_it() {
        let dir = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        adapter(dir.path(), "acme.0linked", "other", 10, Some(folder.path()));
        adapter(dir.path(), "acme.a", "acme", 20, None);
        assert_eq!(extensions_in(dir.path()), ["acme.0linked", "acme.a"]);

        // Edited after it was linked: earlier in both id and link time, yet it claimed acme last.
        let toml = folder.path().join("vorn-extension.toml");
        let edited = std::fs::read_to_string(&toml)
            .unwrap()
            .replace("id = \"other\"", "id = \"acme\"");
        std::fs::write(&toml, edited).unwrap();
        assert_eq!(extensions_in(dir.path()), ["acme.a"]);

        let adapters = Adapters::installed_in(dir.path());
        assert_eq!(adapters.extension_of("acme").as_deref(), Some("acme.a"));
    }

    #[test]
    fn a_link_whose_manifest_changed_since_claims_its_agent_last() {
        let dir = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        adapter(dir.path(), "acme.0linked", "acme", 10, Some(folder.path()));
        adapter(dir.path(), "acme.a", "acme", 20, None);
        assert_eq!(extensions_in(dir.path()), ["acme.0linked"]);

        // Edited away and back to the same agent: no longer the manifest it was linked as.
        let toml = folder.path().join("vorn-extension.toml");
        let edited = std::fs::read_to_string(&toml).unwrap() + "\n# edited\n";
        std::fs::write(&toml, edited).unwrap();
        assert_eq!(extensions_in(dir.path()), ["acme.a"]);
    }
}

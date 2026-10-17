//! The configuration (`config:load`, `config:save`), read and written in
//! `vorn.db` through [`vorn_store`].
//!
//! A save is checked against the revision the client last loaded, so two
//! clients cannot delete each other's rows, then tells every client
//! (`config:changed`). Tasks and workflows are not in the configuration: they
//! have their own calls, and the triggers tasks fire follow from those. The settings that
//! belong to a viewer ([`vorn_store::VIEWER_SETTING_KEYS`]) are also kept per
//! viewer, so a device's own font size and view come back from here and its
//! local storage is only a cache of them.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use tracing::warn;
use vorn_agents::launch::shell as launch_shell;
use vorn_agents::launch::Platform;
use vorn_agents::Agents;
use vorn_store::{Store, StoreOptions};
use vorn_vault::Kind;

use super::{Answer, Native};

/// Every call this module answers.
pub const METHODS: &[&str] = &["config:load", "config:save"];

/// Who is looking: the key their own settings are kept under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Viewer {
    /// The desktop app, by its launch credential.
    Desktop,
    /// A phone or browser, by the id of the device token it signed in with.
    Device(String),
    /// Anything else on this machine: the CLI, an agent's tools.
    Local,
}

impl Viewer {
    /// Who presents `credential`, once it has been admitted.
    pub fn of_credential(credential: &str, desktop: Option<&[u8]>) -> Viewer {
        if desktop.is_some_and(|d| vorn_reach::token::constant_time_eq(credential.as_bytes(), d)) {
            return Viewer::Desktop;
        }
        match vorn_reach::token::parse(credential) {
            Some(parsed) => Viewer::Device(parsed.id.to_owned()),
            None => Viewer::Local,
        }
    }

    /// The key its own settings are kept under; the CLI and agents' tools keep none.
    pub(crate) fn key(&self) -> Option<String> {
        match self {
            Viewer::Desktop => Some("desktop".to_owned()),
            Viewer::Device(id) => Some(format!("token:{id}")),
            Viewer::Local => None,
        }
    }
}

/// The app's defaults a configuration falls back on, `agents` among them.
pub fn options(agents: &Agents) -> StoreOptions {
    let var = |name: &str| std::env::var(name).ok();
    let default_agent_commands = agents
        .iter()
        .map(|agent| {
            let command = agent.default_command();
            let mut entry = Map::new();
            entry.insert("command".into(), json!(command.command));
            entry.insert("args".into(), json!(command.args));
            if let Some(headless) = command.headless_args {
                entry.insert("headlessArgs".into(), json!(headless));
            }
            (agent.id().to_owned(), Value::Object(entry))
        })
        .collect();
    StoreOptions {
        default_shell: launch_shell::default_shell(None, Platform::HOST, var),
        default_agent_commands,
        default_workspace: serde_json::from_value(json!({
            "id": "personal",
            "name": "Personal",
            "icon": "User",
            "iconColor": "#6b7280",
            "order": 0
        }))
        .expect("the default workspace is a workspace"),
        owner_name: "owner".to_owned(),
        seed_workflows: Vec::new(),
    }
}

/// What a save changed: the configuration before and after it.
struct Saved {
    before: Value,
    after: Value,
}

/// Answers `config:load` or `config:save` for `viewer`.
pub async fn answer(native: &Arc<Native>, method: &str, params: Value, viewer: &Viewer) -> Answer {
    // Only a test starts vornd without the database.
    if native.database().is_none() {
        return Answer::Unanswered;
    }
    let (n, key) = (Arc::clone(native), viewer.key());
    match method {
        "config:load" => {
            let loaded = blocking(method, move || {
                with_store(&n, |store| match &key {
                    Some(key) => store.load_config_for(key),
                    None => store.load_config(),
                })
            });
            match loaded.await {
                Ok(config) => Answer::Result(config),
                Err(message) => Answer::Error(message),
            }
        }
        "config:save" => {
            let mut sent = params;
            let passwords = super::credential::take_host_passwords(&mut sent);
            let _order = native.config_order.lock().await;
            let saved = blocking(method, move || {
                for (id, password) in &passwords {
                    n.secrets.put_item(Kind::HostPassword, id, password)?;
                }
                let saved = with_store(&n, |store| save(store, &sent, key.as_deref()))?;
                for id in super::credential::dropped_host_passwords(&saved.before, &saved.after) {
                    n.secrets.remove_item(Kind::HostPassword, &id);
                }
                Ok(saved.after)
            });
            match saved.await {
                Ok(after) => {
                    native.broadcast("config:changed", after);
                    native.config_changed();
                    Answer::Void
                }
                Err(message) => Answer::Error(message),
            }
        }
        _ => Answer::Unanswered,
    }
}

/// Tells every client what changed after a write elsewhere (a workflow, a
/// project, or tasks written without saying which): the configuration
/// (`config:changed`) and the workflows (`workflow:changed`) whole, and that
/// the task list is to be read again (`task:rows`), a page at a time.
pub(crate) async fn announce(native: &Arc<Native>) {
    let n = Arc::clone(native);
    let _order = native.config_order.lock().await;
    let loaded = blocking("config:changed", move || {
        with_store(&n, |s| {
            let config = s.load_config()?;
            let workflows = s.call("dbListWorkflows", json!([]))?;
            Ok((config, workflows))
        })
    })
    .await;
    match loaded {
        Ok((config, workflows)) => {
            native.broadcast("config:changed", config);
            native.broadcast("task:rows", super::task_list::Change::Reload.note());
            native.broadcast("workflow:changed", workflows);
            // Its own call, so a write Tasks makes never waits on Tasks.
            if let Some(tasks) = native.tasks().filter(|t| t.is_on()).cloned() {
                tokio::spawn(async move { tasks.badges().await });
            }
            native.config_changed();
        }
        Err(err) => warn!(%err, "could not read the configuration to announce it"),
    }
}

/// Runs `f` on a blocking thread; a panic is the call's error.
pub(crate) async fn blocking<T: Send + 'static>(
    method: &str,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.unwrap_or_else(|err| {
        warn!(%method, %err, "a configuration call failed");
        Err(format!("{method} failed in vornd"))
    })
}

pub(crate) fn with_store<T>(
    native: &Native,
    f: impl FnOnce(&mut Store) -> vorn_store::Result<T>,
) -> Result<T, String> {
    let db = native.database().ok_or("vornd has no database")?;
    let store = Store::open_beside(db)
        .map_err(|e| e.to_string())?
        .ok_or("the database does not exist yet")?;
    let mut store = store.with_defaults(options(&native.agents()));
    f(&mut store).map_err(|e| e.to_string())
}

fn save(store: &mut Store, config: &Value, viewer: Option<&str>) -> vorn_store::Result<Saved> {
    let before = store.load_config()?;
    store.save_config(config, &[])?;
    let defaults = config.get("defaults").and_then(Value::as_object);
    if let (Some(viewer), Some(defaults)) = (viewer, defaults) {
        store.save_viewer_settings(viewer, defaults)?;
    }
    let after = store.load_config()?;
    Ok(Saved { before, after })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_viewers_apart_by_their_credential() {
        let desktop = b"launch-secret".to_vec();
        assert_eq!(
            Viewer::of_credential("launch-secret", Some(&desktop)),
            Viewer::Desktop
        );
        assert_eq!(
            Viewer::of_credential("vorn_abc_s3cret", Some(&desktop)),
            Viewer::Device("abc".into())
        );
        assert_eq!(Viewer::of_credential("local-token", None), Viewer::Local);
        assert_eq!(
            Viewer::Device("abc".into()).key().as_deref(),
            Some("token:abc")
        );
        assert_eq!(Viewer::Local.key(), None);
    }

    #[test]
    fn the_defaults_name_every_agent_that_is_on() {
        let agents = crate::adapters::testing::adapters().agents();
        let options = options(&agents);
        assert_eq!(
            &options.default_agent_commands["acme"],
            &json!({ "command": "acme", "args": [], "headlessArgs": ["--yes"] })
        );
        assert!(options.default_agent_commands["beta"]
            .get("headlessArgs")
            .is_none());
        assert_eq!(options.default_agent_commands.len(), agents.len());
        assert!(!options.default_shell.is_empty());
    }
}

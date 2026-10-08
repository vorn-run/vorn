//! The configuration (`config:load`, `config:save`), read and written in
//! `vorn.db` through [`vorn_store`].
//!
//! A save is checked against the revision the client last loaded, so two
//! clients cannot delete each other's rows, then fires the task triggers its
//! changes make and tells every client (`config:changed`). The settings that
//! belong to a viewer ([`vorn_store::VIEWER_SETTING_KEYS`]) are also kept per
//! viewer, so a device's own font size and view come back from here and its
//! local storage is only a cache of them.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use tracing::warn;
use vorn_agents::launch::shell as launch_shell;
use vorn_agents::launch::Platform;
use vorn_agents::Agent;
use vorn_store::{Store, StoreOptions};

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
    fn key(&self) -> Option<String> {
        match self {
            Viewer::Desktop => Some("desktop".to_owned()),
            Viewer::Device(id) => Some(format!("token:{id}")),
            Viewer::Local => None,
        }
    }
}

/// The app's defaults a configuration falls back on.
pub fn options() -> StoreOptions {
    let var = |name: &str| std::env::var(name).ok();
    let default_agent_commands = Agent::ALL
        .into_iter()
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
    // A vornd started without the database (a test's) leaves the call to the server.
    if native.database().is_none() {
        return Answer::Forward;
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
            let sent = params.clone();
            let saved = blocking(method, move || {
                with_store(&n, |store| save(store, &sent, key.as_deref()))
            });
            match saved.await {
                Ok(Saved { before, after }) => {
                    fire_triggers(native, &before, &params).await;
                    native.broadcast("config:changed", after);
                    Answer::Void
                }
                Err(message) => Answer::Error(message),
            }
        }
        _ => Answer::Forward,
    }
}

/// Runs `f` on a blocking thread; a panic is the call's error.
async fn blocking<T: Send + 'static>(
    method: &str,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.unwrap_or_else(|err| {
        warn!(%method, %err, "a configuration call failed");
        Err(format!("{method} failed in vornd"))
    })
}

fn with_store<T>(
    native: &Native,
    f: impl FnOnce(&mut Store) -> vorn_store::Result<T>,
) -> Result<T, String> {
    let db = native.database().ok_or("vornd has no database")?;
    let store = Store::open_beside(db)
        .map_err(|e| e.to_string())?
        .ok_or("the database does not exist yet")?;
    let mut store = store.with_defaults(options());
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

/// The task triggers a save fires, read from what the client sent against
/// what was stored before it, and the schedules armed again.
async fn fire_triggers(native: &Native, before: &Value, sent: &Value) {
    let Some(work) = native.work().cloned() else {
        return;
    };
    for trigger in vorn_work::task_triggers::for_change(before, sent) {
        if let Err(err) = work.trigger(&trigger).await {
            warn!(%err, "a task trigger was refused");
        }
    }
    work.workflows_changed_elsewhere();
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
    fn the_defaults_name_every_agent_as_the_app_does() {
        let options = options();
        let claude = &options.default_agent_commands["claude"];
        assert_eq!(
            claude,
            &json!({ "command": "claude", "args": [], "headlessArgs": ["--dangerously-skip-permissions"] })
        );
        assert!(options.default_agent_commands["opencode"]
            .get("headlessArgs")
            .is_none());
        assert_eq!(options.default_agent_commands.len(), Agent::ALL.len());
        assert!(!options.default_shell.is_empty());
    }
}

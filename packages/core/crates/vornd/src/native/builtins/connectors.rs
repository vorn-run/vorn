//! The Connectors built-in in vornd: [`vorn_connectors::builtin`] on the
//! public API, behind its switch. This module only hands the built-in what
//! [`Host`] asks for and refuses its calls while it is off.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use serde_json::{json, Value};
use tracing::{info, warn};
use vorn_connectors::builtin::{self, Answer as BuiltinAnswer, Connectors};
use vorn_connectors::host::{Fields, Host, Pending, Services, TableWork};
use vorn_extensions::gate::{self, Caller, World};
use vorn_extensions::manifest::{file, Manifest};

use super::Switches;
use crate::native::{secrets::Known, Answer, Native};

/// Connectors is on until the person turns it off.
const ON_BY_DEFAULT: bool = true;

/// What a call to Connectors is told while it is off.
pub const OFF: &str = "Connectors is off. Turn it on in Settings → Extensions.";

/// The built-in and its switch.
pub struct ConnectorsBuiltin {
    inner: Arc<Connectors>,
    manifest: Manifest,
    switches: Switches,
    on: AtomicBool,
    polls: Arc<vorn_extensions::polls::Polls>,
}

impl std::fmt::Debug for ConnectorsBuiltin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectorsBuiltin")
            .field("on", &self.on)
            .finish_non_exhaustive()
    }
}

/// The built-in's manifest, which it is held to.
pub fn manifest() -> Manifest {
    file::parse_builtin(builtin::MANIFEST)
        .expect("the Connectors built-in's manifest is checked by its tests")
}

impl ConnectorsBuiltin {
    /// The built-in on `native`, with the bridge's `services`, switched as `switches` keep it.
    pub fn new(
        native: &Arc<Native>,
        services: Services,
        switches: Switches,
    ) -> Arc<ConnectorsBuiltin> {
        let manifest = manifest();
        let host = Arc::new(Wired {
            native: Arc::downgrade(native),
            manifest: manifest.clone(),
        });
        let polls = Arc::clone(&services.polls);
        let on = switches.is_on(builtin::ID, ON_BY_DEFAULT);
        let me = Arc::new(ConnectorsBuiltin {
            inner: Connectors::new(host, services),
            manifest,
            switches,
            on: AtomicBool::new(on),
            polls,
        });
        if on {
            me.polls.offer(builtin::POLL_TRIGGER, builtin::ID);
        }
        me
    }

    /// The built-in itself, whatever its switch says.
    pub fn inner(&self) -> &Arc<Connectors> {
        &self.inner
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    /// As the Extensions list shows it.
    pub fn listed(&self) -> Value {
        json!({
            "id": self.manifest.id,
            "name": self.manifest.name,
            "description": self.manifest.description,
            "on": self.is_on(),
        })
    }

    /// Turns it on or off and keeps the choice. Off stops every connection's
    /// child and every poll, and keeps the connections, their secrets and
    /// their inbox as they are.
    pub async fn set_on(&self, on: bool) {
        self.switches.set(builtin::ID, on);
        if self.on.swap(on, Ordering::AcqRel) == on {
            return;
        }
        if on {
            info!("[connectors] switched on");
            self.polls.offer(builtin::POLL_TRIGGER, builtin::ID);
            self.inner.reconcile().await;
        } else {
            info!("[connectors] switched off; connections are kept");
            self.polls.withdraw(builtin::ID);
            self.inner.stop().await;
        }
    }

    /// Answers one of the built-in's calls; while it is off, only those it answers off.
    pub async fn answer(&self, method: &str, params: Value) -> Answer {
        if !self.is_on() && !builtin::WHILE_OFF.contains(&method) {
            return Answer::Error(OFF.to_owned());
        }
        match self.inner.answer(method, params).await {
            BuiltinAnswer::Result(v) => Answer::Result(v),
            BuiltinAnswer::Void => Answer::Void,
            BuiltinAnswer::Error(e) => Answer::Error(e),
            BuiltinAnswer::Unanswered => Answer::Unanswered,
        }
    }

    /// Settles every installed connector's implicit connection, if it is on.
    pub async fn reconcile(&self) {
        if self.is_on() {
            self.inner.reconcile().await;
        }
    }

    /// Stops every connection's child, as vornd stops.
    pub async fn stop(&self) {
        self.inner.stop().await;
    }
}

/// vornd, as the built-in reaches it.
struct Wired {
    native: Weak<Native>,
    manifest: Manifest,
}

fn stopping() -> String {
    "vornd is stopping".to_owned()
}

impl Host for Wired {
    fn call(&self, method: &str, args: Value) -> Pending<Result<Value, String>> {
        let admitted = gate::admit_call(&self.manifest, method).map(|_| ());
        let world = crate::native::extensions::world::Wired {
            native: self.native.clone(),
        };
        let caller = Caller {
            extension: builtin::ID.to_owned(),
            project: None,
        };
        let method = method.to_owned();
        Box::pin(async move {
            admitted.map_err(|e| e.message)?;
            world
                .call(&caller, &method, args)
                .await
                .map_err(|e| e.message)
        })
    }

    fn tables(&self, work: TableWork) -> Pending<Result<(), String>> {
        let native = self.native.upgrade();
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            tokio::task::spawn_blocking(move || {
                let mut store = native.store().ok_or("vornd has no database")?;
                work(&mut store).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }

    fn secrets(&self, id: &str) -> Pending<Option<Fields>> {
        let (native, id) = (self.native.upgrade(), id.to_owned());
        Box::pin(async move {
            let native = native?;
            tokio::task::spawn_blocking(move || match native.secrets.lookup(&id) {
                Known::Fields(fields) => Some(fields),
                Known::None | Known::Unknown => None,
            })
            .await
            .ok()
            .flatten()
        })
    }

    fn set_secrets(&self, id: &str, fields: Fields) {
        if let Some(native) = self.native.upgrade() {
            native.secrets.set(id, fields);
        }
    }

    fn merge_secret(&self, id: &str, field: &str, value: &str) -> Pending<Result<(), String>> {
        let native = self.native.upgrade();
        let (id, field, value) = (id.to_owned(), field.to_owned(), value.to_owned());
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            tokio::task::spawn_blocking(move || native.secrets.merge(&id, &field, &value))
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn forget_secrets(&self, id: &str) {
        if let Some(native) = self.native.upgrade() {
            native.secrets.forget(id);
        }
    }

    fn child_env(&self) -> Vec<(String, String)> {
        self.native
            .upgrade()
            .map(|n| n.child_env())
            .unwrap_or_default()
    }

    fn find_on_path(&self, name: &str, path: &str) -> Option<PathBuf> {
        crate::native::env::find_on_path(name, path)
    }

    fn github_origin(&self, dir: &Path) -> Option<(String, String)> {
        let native = self.native.upgrade()?;
        let git = vorn_git::repo::Git {
            bin: native.env.git_bin(),
            env: native.env.get(),
            ssh: None,
        };
        git.github_origin(dir).map(|r| (r.owner, r.repo))
    }

    fn changed(&self) {
        if let Some(native) = self.native.upgrade() {
            native.signal_change();
        }
    }

    fn broadcast(&self, method: &str, params: Value) {
        if let Some(native) = self.native.upgrade() {
            native.broadcast(method, params);
        }
    }

    fn import_core_credentials(&self, params: Value) -> Pending<Result<(u64, u64), String>> {
        let native = self.native.upgrade();
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            let n = Arc::clone(&native);
            let imported = tokio::task::spawn_blocking(move || {
                crate::native::config::with_store(&n, |s| {
                    Ok(crate::native::credential::import(&n, s, &params))
                })?
            })
            .await
            .map_err(|e| e.to_string())??;
            if imported.1 > 0 {
                crate::native::config::announce(&native).await;
            }
            Ok(imported)
        })
    }
}

/// The bridge's services for the built-in, started once vornd has a
/// database and its own address.
pub async fn services(native: &Arc<Native>, data_dir: &Path, origin: String) -> Services {
    let proxy = if vorn_extensions::sandbox::available() {
        match vorn_extensions::netproxy::NetProxy::start().await {
            Ok(p) => Some(p),
            Err(err) => {
                warn!("[connectors] sandboxed connectors have no network: {err}");
                None
            }
        }
    } else {
        None
    };
    let home = PathBuf::from(crate::native::shell::home_dir());
    Services {
        lifecycle: Arc::clone(native.lifecycle()),
        installs: native.installs(data_dir),
        windows: Arc::clone(native.windows()),
        polls: Arc::clone(native.polls()),
        proxy,
        origin,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        catalog_cache: home.join(".vorn").join("connector-catalog.json"),
        repo_root: std::env::var_os("VORN_CONNECTORS_ROOT").map(PathBuf::from),
    }
}

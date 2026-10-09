//! What vornd's calls share beyond one connection: its bound address, its scripts and its claims.

use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::Notify;

use crate::claims::Claims;
use crate::native::script::Scripts;

#[derive(Debug, Default)]
pub struct AppLink {
    server_host: Mutex<Option<String>>,
    reached: Notify,
    /// What runs scripts.
    scripts: OnceLock<Arc<Scripts>>,
    /// The conversations being started.
    claims: Claims,
}

impl AppLink {
    /// The address vornd is bound to, `0.0.0.0` when it takes the network; `None` until set.
    pub fn server_host(&self) -> Option<String> {
        self.server_host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Where vornd is bound, set once it listens.
    pub fn set_server_host(&self, host: String) {
        *self.server_host.lock().unwrap_or_else(|e| e.into_inner()) = Some(host);
        self.reached.notify_one();
    }

    /// Resolves each time the address is set, the moment to read again what depends on it.
    pub async fn reached(&self) {
        self.reached.notified().await;
    }

    /// What runs scripts. Only the first one given is kept.
    pub fn set_scripts(&self, scripts: Arc<Scripts>) {
        let _ = self.scripts.set(scripts);
    }

    pub fn scripts(&self) -> Option<&Arc<Scripts>> {
        self.scripts.get()
    }

    /// The conversations being started.
    pub fn claims(&self) -> &Claims {
        &self.claims
    }
}

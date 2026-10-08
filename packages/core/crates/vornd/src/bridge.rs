//! The desktop app's side: what only it can do, such as fetching through a
//! connection's signed-in window. Asked by method name, the same names the
//! desktop answers today (`session:fetch`, `session:check`, `session:forget`,
//! `browser:*`, `device:*`).
//!
//! Until vornd holds the desktop's own connection, the server relays these
//! over the app's channel ([`AppBridge`]).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::applink::AppLink;

/// What a request to the desktop comes to.
pub type Answer<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

/// The desktop, asked by method name.
pub trait Bridge: Send + Sync {
    /// Whether a desktop is there to ask.
    fn connected(&self) -> bool;
    fn request<'a>(&'a self, method: &'a str, params: Value, timeout: Duration) -> Answer<'a>;
}

/// The desktop reached through the server, which holds its connection.
#[derive(Debug)]
pub struct AppBridge(pub Arc<AppLink>);

impl Bridge for AppBridge {
    fn connected(&self) -> bool {
        self.0.listening()
    }

    fn request<'a>(&'a self, method: &'a str, params: Value, timeout: Duration) -> Answer<'a> {
        Box::pin(self.0.ask(method, params, timeout))
    }
}

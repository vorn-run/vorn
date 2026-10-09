//! The desktop app's side: what only it can do, such as fetching through a
//! connection's signed-in window. Asked by method name, the same names the
//! desktop answers (`session:fetch`, `session:check`, `session:forget`,
//! `browser:*`, `device:*`), over the connection its main process claims
//! ([`crate::native::desktop`]).

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::Value;

/// The protocol's calls that only vornd makes, of the desktop; no client
/// sends them, so no group answers them.
pub const ASKED_OF_DESKTOP: &[&str] = &["session:fetch", "session:check", "session:forget"];

/// What a request to the desktop comes to.
pub type Answer<'a> = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'a>>;

/// The desktop, asked by method name.
pub trait Bridge: Send + Sync {
    /// Whether a desktop is there to ask.
    fn connected(&self) -> bool;
    fn request<'a>(&'a self, method: &'a str, params: Value, timeout: Duration) -> Answer<'a>;
}

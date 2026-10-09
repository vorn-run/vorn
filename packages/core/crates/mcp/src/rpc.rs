//! How a tool reaches the Vorn server: the host's half of every call.
//!
//! The TypeScript tools call `rpcCall` and `rpcNotify` from
//! `packages/mcp/src/rpc-client`, which open a socket to the server per call.
//! Here the host decides how a call travels; vornd sends it over one socket to
//! its own `/ws`, so it goes wherever any other client's would.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use serde_json::Value;

/// The ceiling the TypeScript client puts on an ordinary call.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The ceiling for calls that start a connector package, which downloads it first.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

/// A call that failed, worded as the TypeScript client words the same failure,
/// since a tool often passes the message on to the agent as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcError(pub String);

impl RpcError {
    /// A JSON-RPC error answer, as `rpcCall` turns it into an `Error`.
    pub fn answered(message: &str) -> RpcError {
        RpcError(explain(message))
    }

    /// No answer within `timeout`.
    pub fn timed_out(method: &str, timeout: Duration) -> RpcError {
        RpcError(format!(
            "RPC call \"{method}\" timed out after {}ms",
            timeout.as_millis()
        ))
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RpcError {}

impl From<RpcError> for String {
    fn from(err: RpcError) -> String {
        err.0
    }
}

/// `explain` from the TypeScript client: a server without the method is older
/// than the caller, and the message says so.
fn explain(message: &str) -> String {
    let Some(method) = message.strip_prefix("Method not found:") else {
        return message.to_owned();
    };
    format!(
        "This server does not have {}, so it is older than the vorn command asking for it.\n\
         Restart Vorn to pick up the newer server, or run this against the matching build.",
        crate::json::trim(method)
    )
}

/// The Vorn server, as the tools see it.
///
/// `params` of `None` is a call without params, which is what the TypeScript
/// sends for `undefined`. A missing `result` in the answer reads as `null`.
pub trait Rpc: Sync {
    /// A request, answered or failed within `timeout`.
    fn call(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> impl Future<Output = Result<Value, RpcError>> + Send;

    /// A notification: sent, never answered. Fails only when it cannot be sent.
    fn notify(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> impl Future<Output = Result<(), RpcError>> + Send;
}

/// Who is calling: what the TypeScript server reads from its own process.
///
/// It runs as a child of the agent, so `process.cwd()` and `VORN_SESSION_ID`
/// are the agent's. A server shared by every agent has to be told them, which
/// the relay in `packages/mcp` does with each request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caller {
    /// The agent's working directory, absolute.
    pub cwd: String,
    /// The Vorn session the agent runs in, when it runs in one.
    pub session: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_method_reads_as_an_old_server() {
        let err = RpcError::answered("Method not found: artifact:list");
        assert!(err
            .0
            .starts_with("This server does not have artifact:list, so it is older"));
        assert_eq!(RpcError::answered("boom").0, "boom");
    }

    #[test]
    fn a_timeout_names_the_method_and_the_wait() {
        assert_eq!(
            RpcError::timed_out("config:load", DEFAULT_TIMEOUT).0,
            "RPC call \"config:load\" timed out after 10000ms"
        );
    }
}

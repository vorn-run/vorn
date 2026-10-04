//! The napi adapter over `vorn-git`: one async call, `gitRun`, that returns a
//! promise and does the work on a thread of its own, never on Node's.
//!
//! The logic is all in `crates/vorn-git`; this only converts the request,
//! bounds how many commands run at once, and turns an error into the message
//! the JS path's `execFileSync` would have thrown.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use napi_derive::napi;
use tokio::sync::Semaphore;

/// Commands in flight at once. A board refreshing thirty diff panels would
/// otherwise fork thirty gits together; the rest queue here, off the loop.
const MAX_CONCURRENT: usize = 8;

static SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(MAX_CONCURRENT));

#[napi(object)]
pub struct GitRequest {
    /// The git executable, resolved as the JS path resolves it.
    pub bin: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: HashMap<String, String>,
    pub timeout_ms: u32,
    /// Stdout past this many bytes is an error, as `maxBuffer` is for `execFileSync`.
    pub max_buffer: u32,
}

/// Resolves with git's stdout, untrimmed; rejects with the message
/// `execFileSync` would have thrown.
#[napi]
pub async fn git_run(request: GitRequest) -> napi::Result<String> {
    let req = vorn_git::Request {
        bin: request.bin,
        args: request.args,
        cwd: request.cwd.into(),
        env: request.env.into_iter().collect(),
        timeout: Duration::from_millis(u64::from(request.timeout_ms)),
        max_buffer: request.max_buffer as usize,
    };
    let _slot = SLOTS
        .acquire()
        .await
        .map_err(|err| napi::Error::from_reason(err.to_string()))?;
    // A panic in there is caught by tokio and arrives as an error here, rather
    // than unwinding into Node.
    match tokio::task::spawn_blocking(move || vorn_git::run(&req)).await {
        Ok(Ok(reply)) => Ok(reply.stdout),
        Ok(Err(err)) => Err(napi::Error::from_reason(err.to_string())),
        Err(err) => Err(napi::Error::from_reason(format!("vorn-git: {err}"))),
    }
}

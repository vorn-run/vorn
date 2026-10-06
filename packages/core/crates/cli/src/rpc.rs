//! Finding the running server and calling it.
//!
//! The server announces itself in its data directory: `ws-port` says where it
//! listens, `local-token` how to prove this is the same machine. Both are read
//! on every call, never cached: the credential is regenerated each time the
//! server starts, so a cached one would go stale exactly when Vorn restarts.
//!
//! Every call opens a fresh WebSocket with the credential on the upgrade,
//! sends one JSON-RPC request and waits for the answer with its id. That is
//! the TypeScript client's contract, messages included, since scripts and
//! people read them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, http, Message};

use crate::args::js_trim;

/// The file the server writes its WebSocket port to.
pub const WS_PORT_FILENAME: &str = "ws-port";
/// The file the server writes the local credential to.
pub const LOCAL_TOKEN_FILENAME: &str = "local-token";
/// The socket was refused because it presented no credential.
pub const CLOSE_UNAUTHENTICATED: u16 = 4001;
/// The socket was refused because its credential is not this server's.
pub const CLOSE_CREDENTIAL_REJECTED: u16 = 4002;

/// How long a call may take unless `--timeout` says otherwise.
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// A blank value names no directory, wherever it came from: it would resolve
/// to wherever the command happened to be typed.
fn named(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !js_trim(v).is_empty())
}

/// The data directory of the server this client talks to.
#[derive(Debug, Clone)]
pub struct DataDir {
    path: PathBuf,
    /// Whether somebody named it (`--data-dir` or `VORN_DATA_DIR`). Looking for
    /// any Vorn process is a fair answer only for the default directory.
    named: bool,
}

impl DataDir {
    /// `--data-dir`, else `VORN_DATA_DIR`, else `~/.vorn`.
    pub fn resolve(flag: Option<&str>) -> DataDir {
        let env = std::env::var("VORN_DATA_DIR").ok();
        match named(flag).or(named(env.as_deref())) {
            Some(dir) => DataDir {
                path: PathBuf::from(dir),
                named: true,
            },
            None => DataDir {
                path: home_dir().join(".vorn"),
                named: false,
            },
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn port_file(&self) -> PathBuf {
        self.path.join(WS_PORT_FILENAME)
    }

    pub fn token_file(&self) -> PathBuf {
        self.path.join(LOCAL_TOKEN_FILENAME)
    }
}

/// `os.homedir()`.
pub fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// What reading `ws-port` found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PortFile {
    Port(f64),
    Missing,
    Invalid,
}

/// Why a call did not get an answer, worded as the TypeScript client words it.
#[derive(Debug)]
pub enum CallError {
    NoPortFile(PathBuf),
    InvalidPortFile(PathBuf),
    NoCredential(PathBuf),
    /// The connection could not be made: the reason as Node words it.
    Connect(String),
    /// The socket closed before the answer, with this close code.
    Closed {
        code: u16,
        token_file: PathBuf,
    },
    /// The server answered with an error.
    Remote(String),
    TimedOut {
        method: String,
        ms: u64,
    },
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::NoPortFile(file) => f.write_str(&port_file_missing(file)),
            CallError::InvalidPortFile(file) => {
                let remove = if cfg!(windows) {
                    format!("Remove-Item \"{}\"", file.display())
                } else {
                    format!("rm \"{}\"", file.display())
                };
                write!(
                    f,
                    "Vorn port file exists but contains invalid data ({}).\nDelete it and restart Vorn, or overwrite it with the correct port:\n  {remove}",
                    file.display()
                )
            }
            CallError::NoCredential(file) => write!(
                f,
                "Vorn local credential not found ({}).\nThe server writes it on startup and removes it on shutdown, so this usually means\nVorn is not running. Start Vorn (or `vorn server serve`) and try again.\nIf the server runs with --data-dir, pass the same --data-dir here, or set\nVORN_DATA_DIR to that directory -- which is how anything that is not the CLI,\nMCP included, reaches a server that moved.",
                file.display()
            ),
            CallError::Connect(reason) => write!(
                f,
                "Cannot connect to Vorn server: {reason}. Is the app running?"
            ),
            CallError::Closed { code, token_file }
                if *code == CLOSE_UNAUTHENTICATED || *code == CLOSE_CREDENTIAL_REJECTED =>
            {
                write!(
                    f,
                    "A Vorn server on this port refused the credential in {}.\nAnother server is listening on it with its own data directory, which a dev build\nrunning beside the app does. Point at that one with --data-dir (or VORN_DATA_DIR),\nor stop it.",
                    token_file.display()
                )
            }
            CallError::Closed { code, .. } => write!(
                f,
                "The server closed the connection before answering (code {code})."
            ),
            CallError::Remote(message) => f.write_str(&explain(message)),
            CallError::TimedOut { method, ms } => {
                write!(f, "RPC call \"{method}\" timed out after {ms}ms")
            }
        }
    }
}

impl std::error::Error for CallError {}

// Names the file that was actually read: with --data-dir it is not the one
// under ~/.vorn, and a fix-it line pointing at the wrong path is worse than none.
fn port_file_missing(file: &Path) -> String {
    let file = file.display();
    if cfg!(windows) {
        format!(
            "Vorn port file not found ({file}).\nThe app may be running but the port file was deleted (e.g. by another instance shutting down).\nTo fix, in PowerShell, find the Vorn process and its listening port:\n  Get-NetTCPConnection -State Listen -OwningProcess (Get-Process Vorn).Id | Select LocalPort\nThen write the WS port to the file:\n  '{{\"port\":<PORT>,\"pid\":<PID>}}' | Set-Content -Path \"{file}\"\nOr restart Vorn to regenerate it."
        )
    } else {
        format!(
            "Vorn port file not found ({file}).\nThe app may be running but the port file was deleted (e.g. by another instance shutting down).\nTo fix, run:  lsof -iTCP -sTCP:LISTEN -P | grep Vorn\nThen write the WS port (the one on *:<port>) to the file:\n  echo '{{\"port\":<PORT>,\"pid\":<PID>}}' > \"{file}\"\nOr restart Vorn to regenerate it."
        )
    }
}

/// A method the server does not have means the server is older than this
/// command: a packaged app beside a newer checkout.
fn explain(message: &str) -> String {
    match message.strip_prefix("Method not found:") {
        None => message.to_owned(),
        Some(method) => format!(
            "This server does not have {}, so it is older than the vorn command asking for it.\nRestart Vorn to pick up the newer server, or run this against the matching build.",
            js_trim(method)
        ),
    }
}

/// Whether a process with this pid exists. A pid nobody may signal still
/// exists.
#[cfg(unix)]
fn pid_alive(pid: f64) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid as i64) else {
        return false;
    };
    // SAFETY: kill with signal 0 sends nothing; it only checks that the pid
    // exists and may be signalled. It reads no memory of ours.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Without a cheap check, a recorded pid is trusted, which is what the file
/// says anyway.
#[cfg(not(unix))]
fn pid_alive(_pid: f64) -> bool {
    true
}

/// `ws-port`, or what can be found out when it is missing or stale.
pub fn read_port(dir: &DataDir) -> PortFile {
    let Ok(raw) = std::fs::read_to_string(dir.port_file()) else {
        return discover_and_heal(dir);
    };
    let raw = js_trim(&raw);
    if raw.is_empty() {
        return PortFile::Invalid;
    }

    // JSON: {"port": 53829, "pid": 1234}
    if raw.starts_with('{') {
        let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
            return discover_and_heal(dir);
        };
        let port = parsed.get("port").and_then(Value::as_f64);
        let Some(port) = port.filter(|p| p.is_finite() && *p > 0.0) else {
            return PortFile::Invalid;
        };
        if let Some(pid) = parsed.get("pid").and_then(Value::as_f64) {
            if pid.fract() == 0.0 && pid > 0.0 && !pid_alive(pid) {
                // The server that wrote it is gone: the file is stale.
                return discover_and_heal(dir);
            }
        }
        return PortFile::Port(port);
    }

    // Legacy plain-number format: 53829
    match crate::args::parse_int(raw) {
        Some(port) if port.is_finite() && port > 0.0 => PortFile::Port(port),
        _ => PortFile::Invalid,
    }
}

/// Looks for a listening Vorn process, and writes what it finds back to
/// `ws-port`. Only for the default data directory: in a named one it would
/// report somebody else's server.
fn discover_and_heal(dir: &DataDir) -> PortFile {
    if dir.named {
        return PortFile::Missing;
    }
    match discover_port() {
        Some(port) => {
            let _ = std::fs::create_dir_all(dir.path());
            let _ = std::fs::write(dir.port_file(), format!("{{\"port\":{port}}}"));
            PortFile::Port(f64::from(port))
        }
        None => PortFile::Missing,
    }
}

fn run_quietly(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The digits right after the first `marker` in `line`.
fn digits_after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let rest = &line[line.find(marker)? + marker.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

fn port_after(line: &str, marker: &str) -> Option<u16> {
    digits_after(line, marker)?.parse().ok()
}

/// The port a Vorn process listens on, asked of the OS: the wildcard address
/// first, loopback otherwise.
fn discover_port() -> Option<u16> {
    if cfg!(windows) {
        let tasks = run_quietly(
            "tasklist",
            &["/FI", "IMAGENAME eq Vorn.exe", "/FO", "CSV", "/NH"],
        )?;
        let pid = digits_after(&tasks, "\"Vorn.exe\",\"")?;
        let netstat = run_quietly("netstat", &["-ano"])?;
        let mut fallback = None;
        for line in netstat.lines() {
            if !line.contains("LISTENING") || !line.trim().ends_with(pid) {
                continue;
            }
            if let Some(port) = port_after(line, "0.0.0.0:") {
                return Some(port);
            }
            fallback = fallback.or_else(|| port_after(line, "127.0.0.1:"));
        }
        fallback
    } else {
        let lsof = run_quietly("lsof", &["-iTCP", "-sTCP:LISTEN", "-P", "-n"])?;
        let mut fallback = None;
        for line in lsof.lines().filter(|l| l.contains("Vorn")) {
            if let Some(port) = port_after(line, "*:") {
                return Some(port);
            }
            if fallback.is_none() {
                fallback = line
                    .split_whitespace()
                    .find_map(|word| word.rsplit_once(':').and_then(|(_, p)| p.parse().ok()));
            }
        }
        fallback
    }
}

/// The running server's local credential.
pub fn read_local_token(dir: &DataDir) -> Result<String, CallError> {
    match std::fs::read_to_string(dir.token_file()) {
        Ok(token) if !js_trim(&token).is_empty() => Ok(js_trim(&token).to_owned()),
        _ => Err(CallError::NoCredential(dir.token_file())),
    }
}

/// One server, called one socket per call.
#[derive(Debug)]
pub struct Client {
    dir: DataDir,
    /// `--timeout`, applied to every call a command makes.
    timeout_ms: Option<u64>,
    next_id: AtomicU64,
}

/// `setTimeout` clamps a delay that does not fit in 32 bits to one millisecond.
fn timer_ms(ms: u64) -> u64 {
    if ms > i32::MAX as u64 {
        1
    } else {
        ms
    }
}

impl Client {
    pub fn new(dir: DataDir, timeout_ms: Option<u64>) -> Client {
        Client {
            dir,
            timeout_ms,
            next_id: AtomicU64::new(1),
        }
    }

    pub fn data_dir(&self) -> &DataDir {
        &self.dir
    }

    /// Whether a server is announced right now, without opening a socket.
    pub fn is_running(&self) -> bool {
        matches!(read_port(&self.dir), PortFile::Port(_))
    }

    /// Where to connect and how to prove it, resolved together.
    fn connection(&self) -> Result<(http::Request<()>, f64), CallError> {
        let port = match read_port(&self.dir) {
            PortFile::Port(port) => port,
            PortFile::Missing => return Err(CallError::NoPortFile(self.dir.port_file())),
            PortFile::Invalid => return Err(CallError::InvalidPortFile(self.dir.port_file())),
        };
        let token = read_local_token(&self.dir)?;
        let url = format!("ws://127.0.0.1:{}/ws", crate::js::number(port));
        let mut request = url
            .into_client_request()
            .map_err(|_| CallError::Connect("Invalid URL".into()))?;
        let bearer = http::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| CallError::Connect("Invalid credential".into()))?;
        request
            .headers_mut()
            .insert(http::header::AUTHORIZATION, bearer);
        Ok((request, port))
    }

    /// Sends one request and waits for its answer: the `result`, or `null`
    /// when the server sent none.
    pub async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, CallError> {
        let ms = self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let (request, port) = self.connection()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let exchange = self.exchange(request, port, id, method, params);
        match tokio::time::timeout(Duration::from_millis(timer_ms(ms)), exchange).await {
            Ok(answer) => answer,
            Err(_) => Err(CallError::TimedOut {
                method: method.to_owned(),
                ms,
            }),
        }
    }

    async fn exchange(
        &self,
        request: http::Request<()>,
        port: f64,
        id: u64,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, CallError> {
        let (mut ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|err| connect_error(err, port))?;

        let mut frame = serde_json::Map::new();
        frame.insert("jsonrpc".into(), "2.0".into());
        frame.insert("id".into(), id.into());
        frame.insert("method".into(), method.into());
        // `undefined` params are left out of the frame, as JSON.stringify does.
        if let Some(params) = params {
            frame.insert("params".into(), params);
        }
        let text = Value::Object(frame).to_string();
        if let Err(err) = ws.send(Message::text(text)).await {
            return Err(closed_error(err, &self.dir));
        }

        loop {
            let message = match ws.next().await {
                None => return Err(closed(1006, &self.dir)),
                Some(Err(err)) => return Err(closed_error(err, &self.dir)),
                Some(Ok(message)) => message,
            };
            let raw: &[u8] = match &message {
                Message::Text(text) => text.as_bytes(),
                Message::Binary(bytes) => bytes,
                Message::Close(frame) => {
                    let code = frame.as_ref().map_or(1005, |f| u16::from(f.code));
                    return Err(closed(code, &self.dir));
                }
                _ => continue,
            };
            // Broadcasts, notifications and anything that is not JSON are not the answer.
            let Ok(Value::Object(mut answer)) = serde_json::from_slice::<Value>(raw) else {
                continue;
            };
            if answer.get("id") != Some(&Value::from(id)) {
                continue;
            }
            let _ = ws.close(None).await;
            if let Some(error) = answer.get("error").filter(|e| !e.is_null()) {
                let message = crate::js::string(crate::js::field(error, "message"));
                return Err(CallError::Remote(message));
            }
            return Ok(answer.remove("result").unwrap_or(Value::Null));
        }
    }

    /// Sends a notification and does not wait for anything back.
    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), CallError> {
        let (request, port) = self.connection()?;
        let (mut ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|err| connect_error(err, port))?;
        let mut frame = serde_json::Map::new();
        frame.insert("jsonrpc".into(), "2.0".into());
        frame.insert("method".into(), method.into());
        if let Some(params) = params {
            frame.insert("params".into(), params);
        }
        // The frame is on its way once it is flushed; a close that fails after
        // it is nobody's concern.
        ws.send(Message::text(Value::Object(frame).to_string()))
            .await
            .map_err(|err| connect_error(err, port))?;
        let _ = ws.close(None).await;
        Ok(())
    }
}

fn closed(code: u16, dir: &DataDir) -> CallError {
    CallError::Closed {
        code,
        token_file: dir.token_file(),
    }
}

/// A socket that failed after it opened reads as a close without a frame.
fn closed_error(err: tungstenite::Error, dir: &DataDir) -> CallError {
    match err {
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
            closed(1005, dir)
        }
        _ => closed(1006, dir),
    }
}

/// What Node says when a connection is not made.
fn connect_error(err: tungstenite::Error, port: f64) -> CallError {
    let port = crate::js::number(port);
    let reason = match err {
        tungstenite::Error::Http(response) => {
            format!("Unexpected server response: {}", response.status().as_u16())
        }
        tungstenite::Error::Io(io) => match io.kind() {
            std::io::ErrorKind::ConnectionRefused => {
                format!("connect ECONNREFUSED 127.0.0.1:{port}")
            }
            std::io::ErrorKind::ConnectionReset => "socket hang up".into(),
            _ => io.to_string(),
        },
        other => other.to_string(),
    };
    CallError::Connect(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(path: &Path) -> DataDir {
        DataDir {
            path: path.to_owned(),
            named: true,
        }
    }

    #[test]
    fn reads_both_port_file_formats() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dir(tmp.path());
        assert_eq!(read_port(&d), PortFile::Missing);

        std::fs::write(d.port_file(), "  \n").unwrap();
        assert_eq!(read_port(&d), PortFile::Invalid);

        std::fs::write(d.port_file(), "53829\n").unwrap();
        assert_eq!(read_port(&d), PortFile::Port(53829.0));

        std::fs::write(d.port_file(), "{\"port\":4100}").unwrap();
        assert_eq!(read_port(&d), PortFile::Port(4100.0));

        std::fs::write(d.port_file(), "{\"port\":\"4100\"}").unwrap();
        assert_eq!(read_port(&d), PortFile::Invalid);

        // Not JSON after all: read as missing, which is what a failed parse means.
        std::fs::write(d.port_file(), "{port").unwrap();
        assert_eq!(read_port(&d), PortFile::Missing);

        std::fs::write(d.port_file(), "nope").unwrap();
        assert_eq!(read_port(&d), PortFile::Invalid);
    }

    #[cfg(unix)]
    #[test]
    fn treats_a_dead_pid_as_a_stale_file() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dir(tmp.path());
        let me = std::process::id();
        std::fs::write(d.port_file(), format!("{{\"port\":4100,\"pid\":{me}}}")).unwrap();
        assert_eq!(read_port(&d), PortFile::Port(4100.0));
        // Above any pid_max a Linux or macOS kernel allows.
        std::fs::write(d.port_file(), "{\"port\":4100,\"pid\":2147483646}").unwrap();
        assert_eq!(read_port(&d), PortFile::Missing);
    }

    #[test]
    fn explains_a_missing_method_as_an_older_server() {
        let e = CallError::Remote("Method not found: workflow:run".into()).to_string();
        assert!(e.starts_with("This server does not have workflow:run, so it is older"));
        assert_eq!(CallError::Remote("boom".into()).to_string(), "boom");
    }

    #[test]
    fn reads_a_trimmed_credential() {
        let tmp = tempfile::tempdir().unwrap();
        let d = dir(tmp.path());
        assert!(matches!(
            read_local_token(&d),
            Err(CallError::NoCredential(_))
        ));
        std::fs::write(d.token_file(), " secret\n").unwrap();
        assert_eq!(read_local_token(&d).unwrap(), "secret");
    }
}

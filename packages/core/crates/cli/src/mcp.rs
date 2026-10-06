//! `vorn mcp`: Vorn's MCP server, for an agent that speaks MCP over stdio.
//!
//! vornd serves MCP at `/mcp` over Streamable HTTP. An agent configured with a
//! command rather than a URL starts `vorn mcp`, which relays: each line on
//! stdin is one JSON-RPC message, POSTed to `/mcp` with the local credential;
//! each JSON-RPC message in the answer, a JSON body or each event of an SSE
//! stream, is written to stdout as one line. The session id the server issues
//! on `initialize` goes back on every later request, and the session is ended
//! with a DELETE when stdin closes.
//!
//! stdout carries JSON-RPC and nothing else, ever: the agent parses every
//! line. Everything a person should read goes to stderr. A request whose
//! relay fails is answered with a JSON-RPC error, so the agent is not left
//! waiting for an answer that will never come.
//!
//! Vorn restarting does not end the relay. vornd keeps sessions in memory and
//! may come back on another port with another credential, so when it stops
//! taking requests the relay asks the server again where vornd is, with a
//! bounded backoff, opens a new session by replaying the agent's `initialize`,
//! and sends the request again. A tool call vornd may already have run (its
//! exchange broke after it was sent) is never sent twice: it is answered with
//! an error instead. stdin and stdout stay open throughout.
//!
//! Each message is relayed on its own task, so a cancellation can reach the
//! server while a long tool call is still streaming its answer.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::args::ClientArgs;
use crate::exit::ExitCode;
use crate::output::Io;
use crate::rpc::{read_local_token, CallError, Client, DataDir};

pub const MCP_USAGE: &str = "Usage
  vorn mcp [--data-dir <path>] [--timeout <ms>]

Vorn's MCP server over stdio, for an agent that starts its MCP servers as
commands: JSON-RPC lines on stdin, answers on stdout. It relays to vornd,
which serves MCP when Settings > Experimental > Native server is on, and
follows vornd when Vorn restarts.
";

/// The header Streamable HTTP keeps a session in.
const SESSION_HEADER: &str = "mcp-session-id";
const PROTOCOL_HEADER: &str = "mcp-protocol-version";
/// The agent's working directory, percent-encoded, and its Vorn session:
/// what vornd's tools would otherwise read from their own process.
const CWD_HEADER: &str = "vorn-cwd";
const VORN_SESSION_HEADER: &str = "vorn-session-id";
/// How long requests already sent may still answer once stdin has closed.
const DRAIN_GRACE: Duration = Duration::from_secs(30);
/// How long ending the session may take.
const DELETE_TIMEOUT: Duration = Duration::from_secs(5);
/// The JSON-RPC code a relay failure is answered with: an implementation-defined server error.
const RELAY_FAILED: i64 = -32000;
/// How many times one message is sent again after reconnecting before it is failed.
const MAX_RESENDS: usize = 3;

/// Why the relay stopped before stdin closed.
#[derive(Debug)]
pub enum RelayError {
    /// `/mcp` is not there: vornd does not serve MCP.
    NotServing { port: u16 },
    /// stdin could not be read.
    Input(std::io::Error),
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayError::NotServing { port } => write!(
                f,
                "vornd on port {port} does not serve MCP (/mcp answered 404).\nIt is older than this vorn, or the Native server switch is off: turn on\nSettings > Experimental > Native server and restart Vorn."
            ),
            RelayError::Input(err) => write!(f, "could not read stdin: {err}"),
        }
    }
}

impl std::error::Error for RelayError {}

/// Where vornd serves MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub port: u16,
}

/// Reads the credential for each request, so a restarted server's new one is used.
pub type Credential = Arc<dyn Fn() -> Result<String, CallError> + Send + Sync>;

/// Asks where vornd serves MCP now, or says why that is not known. Asked again
/// each time vornd stops taking requests: a restarted vornd may listen on
/// another port.
pub type Locate =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<Endpoint, String>> + Send>> + Send + Sync>;

/// How the relay waits for vornd to come back: the first delay, doubled up to
/// `max` between attempts, and how long one wait may last before the requests
/// waiting on it are failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// The wait after the first failed attempt.
    pub first: Duration,
    /// The longest wait between two attempts.
    pub max: Duration,
    /// How long vornd may stay away before what waits on it is failed.
    pub give_up_after: Duration,
}

impl Default for Backoff {
    /// Long enough for Vorn to restart, short enough that an agent waiting on
    /// a Vorn that is gone hears so within a minute.
    fn default() -> Backoff {
        Backoff {
            first: Duration::from_millis(200),
            max: Duration::from_secs(5),
            give_up_after: Duration::from_secs(60),
        }
    }
}

/// vornd as the relay reaches it: where it is now and how to find it again.
#[derive(Clone)]
pub struct Upstream {
    /// Where vornd serves MCP when the relay starts.
    pub endpoint: Endpoint,
    /// How to find it again after it stops answering.
    pub locate: Locate,
    pub credential: Credential,
    pub backoff: Backoff,
}

/// The session the relay holds with one vornd.
#[derive(Debug, Clone, Default)]
struct Session {
    /// Issued on `initialize`, sent on everything after.
    id: Option<String>,
    /// The protocol version `initialize` settled on, sent on everything after.
    protocol: Option<String>,
}

/// The vornd requests go to, and how the last reconnect went.
#[derive(Debug, Clone)]
struct Link {
    endpoint: Endpoint,
    session: Session,
    /// Reconnects finished so far. A request tried before the latest one
    /// finished takes that one's outcome rather than starting another.
    epoch: u64,
    last_reconnect: Result<(), String>,
}

/// Why one attempt at relaying a message failed.
enum Failure {
    /// vornd did not take the message: it is down, restarting, or no longer
    /// knows the session. Safe to send again once reconnected.
    Unsent(String),
    /// The message was sent but the exchange broke before an answer: vornd
    /// may have acted on it.
    Broken(String),
    /// vornd answered, with an error or something that could not be relayed.
    Final(String),
    /// The first vornd reached has no `/mcp`.
    NotServing(Endpoint),
}

impl Failure {
    fn into_message(self) -> String {
        match self {
            Failure::Unsent(m) | Failure::Broken(m) | Failure::Final(m) => m,
            Failure::NotServing(endpoint) => RelayError::NotServing {
                port: endpoint.port,
            }
            .to_string(),
        }
    }
}

/// The agent's `initialize` and `notifications/initialized`, replayed to open
/// a new session on a restarted vornd.
#[derive(Debug, Default)]
struct Handshake {
    initialize: Option<String>,
    initialized: Option<String>,
}

/// What every relayed message shares.
struct Shared {
    locate: Locate,
    credential: Credential,
    backoff: Backoff,
    link: Mutex<Link>,
    /// Held while reconnecting, so one restart is handled once.
    reconnecting: tokio::sync::Mutex<()>,
    handshake: Mutex<Handshake>,
    /// Whether vornd has answered anything: before that, a 404 means it does
    /// not serve MCP at all.
    reached: AtomicBool,
    lines: mpsc::Sender<String>,
    fatal: mpsc::Sender<RelayError>,
}

/// Every change under these locks is one assignment, so a panic elsewhere
/// cannot leave the value half-written.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn link(&self) -> Link {
        lock(&self.link).clone()
    }

    async fn emit(&self, line: String) {
        // The writer stops only when stdout is gone, and then nobody is reading.
        let _ = self.lines.send(line).await;
    }

    fn fail(&self, err: RelayError) {
        let _ = self.fatal.try_send(err);
    }

    /// Finds vornd again and opens a new session with it, unless a reconnect
    /// finished since the caller's attempt at `epoch`, whose outcome then stands.
    async fn reconnect(&self, epoch: u64) -> Result<(), String> {
        let _turn = self.reconnecting.lock().await;
        {
            let link = lock(&self.link);
            if link.epoch != epoch {
                return link.last_reconnect.clone();
            }
        }
        let outcome = self.wait_for_vornd().await;
        let mut link = lock(&self.link);
        link.epoch += 1;
        link.last_reconnect = match outcome {
            Ok((endpoint, session)) => {
                link.endpoint = endpoint;
                link.session = session;
                Ok(())
            }
            Err(reason) => Err(reason),
        };
        link.last_reconnect.clone()
    }

    /// Tries to reopen the session until it works or the backoff gives up.
    async fn wait_for_vornd(&self) -> Result<(Endpoint, Session), String> {
        let deadline = Instant::now() + self.backoff.give_up_after;
        let mut delay = self.backoff.first;
        loop {
            match self.reopen().await {
                Ok(opened) => {
                    eprintln!("vorn: reconnected to vornd on port {}", opened.0.port);
                    return Ok(opened);
                }
                Err(reason) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(format!(
                            "vornd did not come back within {}s: {reason}",
                            self.backoff.give_up_after.as_secs_f64()
                        ));
                    }
                    tokio::time::sleep(delay.min(deadline - now)).await;
                    delay = delay.saturating_mul(2).min(self.backoff.max);
                }
            }
        }
    }

    /// Asks where vornd is and, if the agent had opened a session, opens a
    /// new one there as the agent did.
    async fn reopen(&self) -> Result<(Endpoint, Session), String> {
        let endpoint = (self.locate)().await?;
        let (initialize, initialized) = {
            let handshake = lock(&self.handshake);
            (handshake.initialize.clone(), handshake.initialized.clone())
        };
        let Some(initialize) = initialize else {
            return Ok((endpoint, Session::default()));
        };
        let token = (self.credential)().map_err(|err| err.to_string())?;
        let req = request(
            endpoint,
            &Session::default(),
            Method::POST,
            initialize.into(),
            &token,
        )?;
        let response = send(endpoint, req).await.map_err(Failure::into_message)?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("vornd answered {status} to initialize"));
        }
        let id = header(&response, SESSION_HEADER);
        let mut protocol = None;
        for message in messages(response).await? {
            let value: Value = serde_json::from_str(&message).unwrap_or(Value::Null);
            if let Some(error) = value.get("error") {
                return Err(format!("vornd refused initialize: {error}"));
            }
            protocol = protocol_version(&value).or(protocol);
        }
        let session = Session { id, protocol };
        if let Some(note) = initialized {
            let req = request(endpoint, &session, Method::POST, note.into(), &token)?;
            let status = send(endpoint, req)
                .await
                .map_err(Failure::into_message)?
                .status();
            if !status.is_success() {
                return Err(format!(
                    "vornd answered {status} to notifications/initialized"
                ));
            }
        }
        Ok((endpoint, session))
    }
}

/// One JSON-RPC message from a body or an event, as one line, if it is JSON.
///
/// A raw line break can only be whitespace in valid JSON (inside a string it
/// must be escaped), so removing them keeps the message and makes it one line.
fn one_line(data: &str) -> Option<String> {
    let line: String = data.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    let line = line.trim();
    if line.is_empty() || serde_json::from_str::<Value>(line).is_err() {
        return None;
    }
    Some(line.to_owned())
}

/// What a line from the agent is, as far as relaying it needs to know.
struct Outgoing {
    /// The id to answer a failure with, when it is a request.
    id: Option<Value>,
    initialize: bool,
    initialized: bool,
    /// Whether vornd acting on it twice is harmless: anything but a tool call.
    resendable: bool,
}

fn outgoing(line: &str) -> Outgoing {
    let parsed: Option<Value> = serde_json::from_str(line).ok();
    let method = parsed
        .as_ref()
        .and_then(|v| v.get("method"))
        .and_then(Value::as_str);
    let id = parsed
        .as_ref()
        .filter(|_| method.is_some())
        .and_then(|v| v.get("id"))
        .filter(|id| !id.is_null())
        .cloned();
    Outgoing {
        id,
        initialize: method == Some("initialize"),
        initialized: method == Some("notifications/initialized"),
        resendable: method != Some("tools/call"),
    }
}

fn error_line(id: &Value, message: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": RELAY_FAILED, "message": message },
    })
    .to_string()
}

fn protocol_version(message: &Value) -> Option<String> {
    message
        .pointer("/result/protocolVersion")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn header(response: &Response<Incoming>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// One HTTP exchange with vornd, on its own connection. Tells a request that
/// never left from one that broke after it was sent.
async fn send(
    endpoint: Endpoint,
    request: Request<Full<Bytes>>,
) -> Result<Response<Incoming>, Failure> {
    let stream = TcpStream::connect(("127.0.0.1", endpoint.port))
        .await
        .map_err(|err| {
            Failure::Unsent(format!(
                "could not connect to vornd on port {}: {err}",
                endpoint.port
            ))
        })?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|err| Failure::Unsent(format!("could not talk to vornd: {err}")))?;
    // Driven until the answer has been read; ends with it.
    tokio::spawn(connection);
    sender.try_send_request(request).await.map_err(|mut err| {
        let message = format!("vornd did not answer: {}", err.error());
        match err.take_message() {
            Some(_) => Failure::Unsent(message),
            None => Failure::Broken(message),
        }
    })
}

fn request(
    endpoint: Endpoint,
    session: &Session,
    method: Method,
    body: Bytes,
    token: &str,
) -> Result<Request<Full<Bytes>>, String> {
    let mut builder = Request::builder()
        .method(method)
        .uri("/mcp")
        .header(hyper::header::HOST, format!("127.0.0.1:{}", endpoint.port))
        .header(hyper::header::ACCEPT, "application/json, text/event-stream")
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .header(hyper::header::AUTHORIZATION, format!("Bearer {token}"));
    if let Some(id) = &session.id {
        builder = builder.header(SESSION_HEADER, id);
    }
    if let Some(protocol) = &session.protocol {
        builder = builder.header(PROTOCOL_HEADER, protocol);
    }
    if let Ok(cwd) = std::env::current_dir() {
        builder = builder.header(CWD_HEADER, encode_uri_component(&cwd.to_string_lossy()));
    }
    if let Some(session) = std::env::var("VORN_SESSION_ID")
        .ok()
        .filter(|s| !s.is_empty())
    {
        builder = builder.header(VORN_SESSION_HEADER, session);
    }
    builder
        .body(Full::new(body))
        .map_err(|err| format!("could not build the request: {err}"))
}

/// `encodeURIComponent`: every byte but the unreserved ones escaped, so any
/// path fits in a header.
fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Writes one message from the server, remembering the protocol version an
/// `initialize` answer settles on.
async fn deliver(shared: &Shared, data: &str, initialize: bool) {
    let Some(line) = one_line(data) else {
        if !data.trim().is_empty() {
            eprintln!("vorn: vornd sent something that is not JSON; dropped it");
        }
        return;
    };
    if initialize {
        let version = serde_json::from_str::<Value>(&line)
            .ok()
            .as_ref()
            .and_then(protocol_version);
        if version.is_some() {
            lock(&shared.link).session.protocol = version;
        }
    }
    shared.emit(line).await;
}

/// An SSE body read as it arrives: the data of each `message` event.
#[derive(Debug, Default)]
struct Events {
    pending: Vec<u8>,
    data: String,
    event: String,
}

impl Events {
    /// Takes the next chunk and returns the data of every event it completes.
    fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut done = Vec::new();
        self.pending.extend_from_slice(chunk);
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&raw[..end]);
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.is_empty() {
                if !self.data.is_empty() && (self.event.is_empty() || self.event == "message") {
                    done.push(std::mem::take(&mut self.data));
                }
                self.data.clear();
                self.event.clear();
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line, ""),
            };
            match field {
                "data" => {
                    if !self.data.is_empty() {
                        self.data.push('\n');
                    }
                    self.data.push_str(value);
                }
                "event" => self.event = value.to_owned(),
                // Comments (`:`), `id` and `retry` say nothing to relay.
                _ => {}
            }
        }
        done
    }
}

fn is_stream(response: &Response<Incoming>) -> bool {
    header(response, hyper::header::CONTENT_TYPE.as_str())
        .is_some_and(|ct| ct.trim_start().starts_with("text/event-stream"))
}

/// Every message of an answer, read whole.
async fn messages(response: Response<Incoming>) -> Result<Vec<String>, String> {
    let stream = is_stream(&response);
    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|err| format!("vornd's answer broke off: {err}"))?
        .to_bytes();
    if stream {
        return Ok(Events::default().feed(&bytes));
    }
    Ok(vec![String::from_utf8_lossy(&bytes).into_owned()])
}

/// The events of an SSE body, each delivered as it completes.
async fn relay_events(shared: &Shared, mut body: Incoming, initialize: bool) -> Result<(), String> {
    let mut events = Events::default();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|err| format!("the event stream broke: {err}"))?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        for data in events.feed(&chunk) {
            deliver(shared, &data, initialize).await;
        }
    }
    Ok(())
}

/// Relays one line from the agent and everything the server answers it with.
async fn relay_one(shared: Arc<Shared>, line: String) {
    let outgoing = outgoing(&line);
    if let Err(message) = relay_until_answered(&shared, &line, &outgoing).await {
        eprintln!("vorn: {message}");
        if let Some(id) = &outgoing.id {
            shared.emit(error_line(id, &message)).await;
        }
    }
}

/// Relays a message, reconnecting and sending it again while vornd has not
/// taken it, or while acting on it twice is harmless.
async fn relay_until_answered(
    shared: &Shared,
    line: &str,
    outgoing: &Outgoing,
) -> Result<(), String> {
    let mut resends = 0;
    loop {
        let link = shared.link();
        let reason = match relay_exchange(shared, &link, line, outgoing.initialize).await {
            Ok(()) => {
                let mut handshake = lock(&shared.handshake);
                if outgoing.initialize {
                    handshake.initialize = Some(line.to_owned());
                } else if outgoing.initialized {
                    handshake.initialized = Some(line.to_owned());
                }
                return Ok(());
            }
            Err(Failure::NotServing(endpoint)) => {
                let err = RelayError::NotServing {
                    port: endpoint.port,
                };
                let message = err.to_string();
                shared.fail(err);
                return Err(message);
            }
            Err(Failure::Final(message)) => return Err(message),
            Err(Failure::Broken(message)) if !outgoing.resendable => return Err(message),
            Err(Failure::Unsent(reason) | Failure::Broken(reason)) => reason,
        };
        if resends == MAX_RESENDS {
            return Err(format!(
                "vornd kept turning the request away after reconnecting: {reason}"
            ));
        }
        resends += 1;
        eprintln!("vorn: {reason}; waiting for vornd");
        shared.reconnect(link.epoch).await?;
    }
}

async fn relay_exchange(
    shared: &Shared,
    link: &Link,
    line: &str,
    initialize: bool,
) -> Result<(), Failure> {
    let token = (shared.credential)().map_err(|err| Failure::Unsent(err.to_string()))?;
    // An `initialize` opens a new session, whatever the relay held.
    let fresh = Session::default();
    let session = if initialize { &fresh } else { &link.session };
    let req = request(
        link.endpoint,
        session,
        Method::POST,
        Bytes::copy_from_slice(line.as_bytes()),
        &token,
    )
    .map_err(Failure::Final)?;
    let response = send(link.endpoint, req).await?;
    let status = response.status();

    let reached = shared.reached.load(Ordering::Relaxed);
    match status {
        StatusCode::NOT_FOUND if session.id.is_some() => {
            return Err(Failure::Unsent(
                "vornd no longer knows this MCP session; Vorn restarted".to_owned(),
            ));
        }
        StatusCode::NOT_FOUND if !reached => return Err(Failure::NotServing(link.endpoint)),
        // A credential refused before any restart is wrong, not stale: said at once below.
        StatusCode::UNAUTHORIZED if !reached => {}
        // vornd without MCP after a restart, with no credential yet, or with a newer one.
        StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED | StatusCode::SERVICE_UNAVAILABLE => {
            return Err(Failure::Unsent(format!(
                "vornd on port {} answered {status}",
                link.endpoint.port
            )));
        }
        _ => shared.reached.store(true, Ordering::Relaxed),
    }
    if status == StatusCode::ACCEPTED {
        return Ok(());
    }
    if !status.is_success() {
        let body = response
            .into_body()
            .collect()
            .await
            .map(|b| String::from_utf8_lossy(&b.to_bytes()).trim().to_owned())
            .unwrap_or_default();
        let detail = if body.is_empty() {
            String::new()
        } else {
            format!(": {body}")
        };
        return Err(Failure::Final(format!("vornd answered {status}{detail}")));
    }

    if initialize {
        if let Some(id) = header(&response, SESSION_HEADER) {
            lock(&shared.link).session = Session {
                id: Some(id),
                protocol: None,
            };
        }
    }

    let stream = is_stream(&response);
    let body = response.into_body();
    if stream {
        return relay_events(shared, body, initialize)
            .await
            .map_err(Failure::Final);
    }
    let bytes = body
        .collect()
        .await
        .map_err(|err| Failure::Final(format!("vornd's answer broke off: {err}")))?
        .to_bytes();
    deliver(shared, &String::from_utf8_lossy(&bytes), initialize).await;
    Ok(())
}

/// Ends the session, if there is one. vornd may not allow it (405); that is
/// its answer to give.
async fn end_session(shared: &Shared) {
    let link = shared.link();
    if link.session.id.is_none() {
        return;
    }
    let Ok(token) = (shared.credential)() else {
        return;
    };
    let Ok(req) = request(
        link.endpoint,
        &link.session,
        Method::DELETE,
        Bytes::new(),
        &token,
    ) else {
        return;
    };
    let _ = tokio::time::timeout(DELETE_TIMEOUT, send(link.endpoint, req)).await;
}

/// Relays `input` to vornd's `/mcp` and its answers to `output` until `input`
/// ends, then ends the session. vornd going away does not end it.
pub async fn relay<R, W>(upstream: Upstream, input: R, mut output: W) -> Result<(), RelayError>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (lines_tx, mut lines_rx) = mpsc::channel::<String>(64);
    let (fatal_tx, mut fatal_rx) = mpsc::channel::<RelayError>(1);
    let writer = tokio::spawn(async move {
        while let Some(line) = lines_rx.recv().await {
            let mut framed = line.into_bytes();
            framed.push(b'\n');
            if output.write_all(&framed).await.is_err() || output.flush().await.is_err() {
                break;
            }
        }
    });

    let shared = Arc::new(Shared {
        locate: upstream.locate,
        credential: upstream.credential,
        backoff: upstream.backoff,
        link: Mutex::new(Link {
            endpoint: upstream.endpoint,
            session: Session::default(),
            epoch: 0,
            last_reconnect: Ok(()),
        }),
        reconnecting: tokio::sync::Mutex::new(()),
        handshake: Mutex::new(Handshake::default()),
        reached: AtomicBool::new(false),
        lines: lines_tx,
        fatal: fatal_tx,
    });

    let mut tasks = JoinSet::new();
    let mut input = input.lines();
    let outcome = loop {
        tokio::select! {
            biased;
            Some(err) = fatal_rx.recv() => break Err(err),
            line = input.next_line() => match line {
                Ok(Some(line)) => {
                    if !line.trim().is_empty() {
                        tasks.spawn(relay_one(shared.clone(), line));
                    }
                }
                Ok(None) => break Ok(()),
                Err(err) => break Err(RelayError::Input(err)),
            },
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
        }
    };

    let outcome = match outcome {
        Ok(()) => {
            // Answers to what was already sent still arrive, for a while.
            let drained = async { while tasks.join_next().await.is_some() {} };
            tokio::select! {
                Some(err) = fatal_rx.recv() => Err(err),
                () = drained => Ok(()),
                () = tokio::time::sleep(DRAIN_GRACE) => Ok(()),
            }
            // A task that failed fatally and then finished, all before the select looked.
            .and_then(|()| fatal_rx.try_recv().map_or(Ok(()), Err))
        }
        Err(err) => Err(err),
    };
    tasks.abort_all();
    if outcome.is_ok() {
        end_session(&shared).await;
    }

    // Everything queued reaches stdout before this returns.
    drop(shared);
    while tasks.join_next().await.is_some() {}
    let _ = writer.await;
    outcome
}

/// Asks the server, through `client`, where vornd serves MCP.
async fn locate(client: &Client) -> Result<Endpoint, String> {
    let status = client
        .call("server:vornd", None)
        .await
        .map_err(|err| format!("could not ask the server where vornd serves MCP: {err}"))?;
    serving(&status)
}

/// `vorn mcp`: asks the server where vornd serves MCP, then relays stdio to it.
pub async fn run(args: &ClientArgs, io: &mut dyn Io) -> ExitCode {
    if args.help {
        io.write(MCP_USAGE);
        return ExitCode::Ok;
    }
    let dir = DataDir::resolve(args.data_dir.as_deref());
    let timeout = args.timeout_ms.map(|ms| ms.min(u64::MAX as f64) as u64);
    let client = Arc::new(Client::new(dir.clone(), timeout));

    let endpoint = match locate(&client).await {
        Ok(endpoint) => endpoint,
        Err(message) => {
            io.write_err(&format!("vorn: {message}\n"));
            return ExitCode::Unreachable;
        }
    };
    // The client reads the port file and the credential on every call, so a
    // restarted server is found wherever it now listens.
    let locator: Locate = Arc::new(move || {
        let client = client.clone();
        Box::pin(async move { locate(&client).await })
    });
    let upstream = Upstream {
        endpoint,
        locate: locator,
        credential: Arc::new(move || read_local_token(&dir)),
        backoff: Backoff::default(),
    };
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    match relay(upstream, stdin, tokio::io::stdout()).await {
        Ok(()) => ExitCode::Ok,
        Err(err) => {
            io.write_err(&format!("vorn: {err}\n"));
            match err {
                RelayError::NotServing { .. } => ExitCode::Unreachable,
                RelayError::Input(_) => ExitCode::Failure,
            }
        }
    }
}

/// Where vornd serves MCP, from `server:vornd`, or why it does not.
pub fn serving(status: &Value) -> Result<Endpoint, String> {
    let state = status.get("state").and_then(Value::as_str);
    match state {
        Some("on") => {}
        Some("failed") => {
            let detail = crate::js::string(status.get("detail"));
            return Err(format!(
                "vornd could not be started, so there is no MCP endpoint to relay to: {detail}"
            ));
        }
        _ => {
            return Err(
                "this Vorn server is not running vornd, so there is no MCP endpoint to relay to."
                    .into(),
            )
        }
    }
    if status.get("nativeServer").and_then(Value::as_bool) != Some(true) {
        return Err("vornd runs without the Native server switch, so it does not serve MCP.\nTurn on Settings > Experimental > Native server and restart Vorn.".into());
    }
    let port = status
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p > 0)
        .ok_or_else(|| "the server named no port for vornd.".to_owned())?;
    Ok(Endpoint { port })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_directory_is_encoded_as_encode_uri_component_encodes_it() {
        assert_eq!(
            encode_uri_component("/home/me/my app"),
            "%2Fhome%2Fme%2Fmy%20app"
        );
        assert_eq!(
            encode_uri_component("/café/a-b_c.d"),
            "%2Fcaf%C3%A9%2Fa-b_c.d"
        );
    }

    #[test]
    fn makes_one_line_of_a_message_and_drops_what_is_not_json() {
        assert_eq!(
            one_line("{\n  \"a\": \"x y\"\r\n}\n").as_deref(),
            Some("{  \"a\": \"x y\"}")
        );
        assert_eq!(one_line("not json"), None);
        assert_eq!(one_line("  "), None);
    }

    #[test]
    fn answers_requests_but_not_notifications() {
        let req = outgoing(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#);
        assert_eq!(req.id, Some(json!(7)));
        assert!(!req.initialize);
        let note = outgoing(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert_eq!(note.id, None);
        let reply = outgoing(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#);
        assert_eq!(reply.id, None);
        assert!(outgoing(r#"{"id":1,"method":"initialize"}"#).initialize);
        assert!(!outgoing(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call"}"#).resendable);
        assert!(outgoing(r#"{"jsonrpc":"2.0","id":4,"method":"tools/list"}"#).resendable);
    }

    #[test]
    fn completes_an_event_split_across_chunks() {
        let mut events = Events::default();
        assert!(events.feed(b"event: message\r\ndata: {\"a\"").is_empty());
        assert!(events.feed(b":1}\r\n").is_empty());
        assert_eq!(events.feed(b"\r\ndata: 2\n\n"), ["{\"a\":1}", "2"]);
        assert!(events.feed(b"event: other\ndata: x\n\n").is_empty());
    }

    #[test]
    fn reads_where_vornd_serves() {
        let on = json!({"state": "on", "port": 4123, "nativeServer": true});
        assert_eq!(serving(&on).unwrap().port, 4123);
        let forward_only = json!({"state": "on", "port": 4123, "nativeServer": false});
        assert!(serving(&forward_only)
            .unwrap_err()
            .contains("Native server"));
        assert!(serving(&json!({"state": "off"}))
            .unwrap_err()
            .contains("not running vornd"));
        assert!(serving(&json!({"state": "failed", "detail": "no binary"}))
            .unwrap_err()
            .ends_with("no binary"));
    }
}

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
//! Each message is relayed on its own task, so a cancellation can reach the
//! server while a long tool call is still streaming its answer.

use std::sync::{Arc, Mutex};
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

use crate::args::ClientArgs;
use crate::exit::ExitCode;
use crate::output::Io;
use crate::rpc::{read_local_token, CallError, Client, DataDir};

pub const MCP_USAGE: &str = "Usage
  vorn mcp [--data-dir <path>] [--timeout <ms>]

Vorn's MCP server over stdio, for an agent that starts its MCP servers as
commands: JSON-RPC lines on stdin, answers on stdout. It relays to vornd,
which serves MCP when Settings > Experimental > Native server is on.
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

/// Why the relay stopped before stdin closed.
#[derive(Debug)]
pub enum RelayError {
    /// `/mcp` is not there: vornd does not serve MCP.
    NotServing { port: u16 },
    /// The server no longer knows the session it issued (it restarted).
    SessionGone,
    /// The local credential is gone: the server stopped.
    Credential(CallError),
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
            RelayError::SessionGone => f.write_str(
                "vornd no longer knows this MCP session (/mcp answered 404); Vorn restarted.\nStart the MCP server again to open a new one.",
            ),
            RelayError::Credential(err) => write!(f, "{err}"),
            RelayError::Input(err) => write!(f, "could not read stdin: {err}"),
        }
    }
}

impl std::error::Error for RelayError {}

/// Where vornd serves MCP.
#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    pub port: u16,
}

/// Reads the credential for each request, so a restarted server's new one is used.
pub type Credential = Arc<dyn Fn() -> Result<String, CallError> + Send + Sync>;

/// What every relayed message shares.
struct Shared {
    endpoint: Endpoint,
    credential: Credential,
    /// Issued on `initialize`, sent on everything after.
    session: Mutex<Option<String>>,
    /// The protocol version `initialize` settled on, sent on everything after.
    protocol: Mutex<Option<String>>,
    lines: mpsc::Sender<String>,
    fatal: mpsc::Sender<RelayError>,
}

impl Shared {
    fn session(&self) -> Option<String> {
        self.session.lock().map(|s| s.clone()).unwrap_or(None)
    }

    fn protocol(&self) -> Option<String> {
        self.protocol.lock().map(|p| p.clone()).unwrap_or(None)
    }

    async fn emit(&self, line: String) {
        // The writer stops only when stdout is gone, and then nobody is reading.
        let _ = self.lines.send(line).await;
    }

    fn fail(&self, err: RelayError) {
        let _ = self.fatal.try_send(err);
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

/// One HTTP exchange with vornd, on its own connection.
async fn send(
    endpoint: Endpoint,
    request: Request<Full<Bytes>>,
) -> Result<Response<Incoming>, String> {
    let stream = TcpStream::connect(("127.0.0.1", endpoint.port))
        .await
        .map_err(|err| {
            format!(
                "could not connect to vornd on port {}: {err}",
                endpoint.port
            )
        })?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|err| format!("could not talk to vornd: {err}"))?;
    // Driven until the answer has been read; ends with it.
    tokio::spawn(connection);
    sender
        .send_request(request)
        .await
        .map_err(|err| format!("vornd did not answer: {err}"))
}

fn request(
    shared: &Shared,
    method: Method,
    body: Bytes,
    token: &str,
) -> Result<Request<Full<Bytes>>, String> {
    let mut builder = Request::builder()
        .method(method)
        .uri("/mcp")
        .header(
            hyper::header::HOST,
            format!("127.0.0.1:{}", shared.endpoint.port),
        )
        .header(hyper::header::ACCEPT, "application/json, text/event-stream")
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .header(hyper::header::AUTHORIZATION, format!("Bearer {token}"));
    if let Some(session) = shared.session() {
        builder = builder.header(SESSION_HEADER, session);
    }
    if let Some(protocol) = shared.protocol() {
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
        let version = serde_json::from_str::<Value>(&line).ok().and_then(|v| {
            v.pointer("/result/protocolVersion")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        if let (Some(version), Ok(mut protocol)) = (version, shared.protocol.lock()) {
            *protocol = Some(version);
        }
    }
    shared.emit(line).await;
}

/// The events of an SSE body, each delivered as it completes.
async fn relay_events(shared: &Shared, mut body: Incoming, initialize: bool) -> Result<(), String> {
    let mut pending: Vec<u8> = Vec::new();
    let mut data = String::new();
    let mut event = String::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|err| format!("the event stream broke: {err}"))?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&raw[..end]);
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.is_empty() {
                if !data.is_empty() && (event.is_empty() || event == "message") {
                    deliver(shared, &data, initialize).await;
                }
                data.clear();
                event.clear();
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line, ""),
            };
            match field {
                "data" => {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(value);
                }
                "event" => event = value.to_owned(),
                // Comments (`:`), `id` and `retry` say nothing to relay.
                _ => {}
            }
        }
    }
    Ok(())
}

/// Relays one line from the agent and everything the server answers it with.
async fn relay_one(shared: Arc<Shared>, line: String) {
    let outgoing = outgoing(&line);
    if let Err(message) = relay_exchange(&shared, line, outgoing.initialize).await {
        eprintln!("vorn: {message}");
        if let Some(id) = &outgoing.id {
            shared.emit(error_line(id, &message)).await;
        }
    }
}

async fn relay_exchange(shared: &Shared, line: String, initialize: bool) -> Result<(), String> {
    let token = match (shared.credential)() {
        Ok(token) => token,
        Err(err) => {
            let message = err.to_string();
            shared.fail(RelayError::Credential(err));
            return Err(message);
        }
    };
    let had_session = shared.session().is_some();
    let req = request(shared, Method::POST, Bytes::from(line), &token)?;
    let response = send(shared.endpoint, req).await?;
    let status = response.status();

    if status == StatusCode::NOT_FOUND {
        let err = if had_session {
            RelayError::SessionGone
        } else {
            RelayError::NotServing {
                port: shared.endpoint.port,
            }
        };
        let message = err.to_string();
        shared.fail(err);
        return Err(message);
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
        return Err(format!("vornd answered {status}{detail}"));
    }

    if let Some(session) = response
        .headers()
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(mut current) = shared.session.lock() {
            *current = Some(session.to_owned());
        }
    }

    let is_stream = response
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.trim_start().starts_with("text/event-stream"));
    let body = response.into_body();
    if is_stream {
        return relay_events(shared, body, initialize).await;
    }
    let bytes = body
        .collect()
        .await
        .map_err(|err| format!("vornd's answer broke off: {err}"))?
        .to_bytes();
    deliver(shared, &String::from_utf8_lossy(&bytes), initialize).await;
    Ok(())
}

/// Ends the session, if there is one. vornd may not allow it (405); that is
/// its answer to give.
async fn end_session(shared: &Shared) {
    if shared.session().is_none() {
        return;
    }
    let Ok(token) = (shared.credential)() else {
        return;
    };
    let Ok(req) = request(shared, Method::DELETE, Bytes::new(), &token) else {
        return;
    };
    let _ = tokio::time::timeout(DELETE_TIMEOUT, send(shared.endpoint, req)).await;
}

/// Relays `input` to vornd's `/mcp` and its answers to `output` until `input`
/// ends, then ends the session.
pub async fn relay<R, W>(
    endpoint: Endpoint,
    credential: Credential,
    input: R,
    mut output: W,
) -> Result<(), RelayError>
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
        endpoint,
        credential,
        session: Mutex::new(None),
        protocol: Mutex::new(None),
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

/// `vorn mcp`: asks the server where vornd serves MCP, then relays stdio to it.
pub async fn run(args: &ClientArgs, io: &mut dyn Io) -> ExitCode {
    if args.help {
        io.write(MCP_USAGE);
        return ExitCode::Ok;
    }
    let dir = DataDir::resolve(args.data_dir.as_deref());
    let timeout = args.timeout_ms.map(|ms| ms.min(u64::MAX as f64) as u64);
    let client = Client::new(dir.clone(), timeout);

    let status = match client.call("server:vornd", None).await {
        Ok(status) => status,
        Err(err) => {
            io.write_err(&format!(
                "vorn: could not ask the server where vornd serves MCP: {err}\n"
            ));
            return ExitCode::Unreachable;
        }
    };
    let endpoint = match serving(&status) {
        Ok(endpoint) => endpoint,
        Err(message) => {
            io.write_err(&format!("vorn: {message}\n"));
            return ExitCode::Unreachable;
        }
    };

    let credential: Credential = Arc::new(move || read_local_token(&dir));
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    match relay(endpoint, credential, stdin, tokio::io::stdout()).await {
        Ok(()) => ExitCode::Ok,
        Err(err) => {
            io.write_err(&format!("vorn: {err}\n"));
            match err {
                RelayError::NotServing { .. } => ExitCode::Unreachable,
                _ => ExitCode::Failure,
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

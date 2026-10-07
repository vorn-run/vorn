//! The endpoint: HTTP and WebSocket in front of the Node server.
//!
//! Clients connect to vornd exactly as they would to the server. A WebSocket
//! upgrade is answered only after the server has accepted the same upgrade, so a
//! refusal (a bad Origin, a wrong token) reaches the client as the server gave
//! it. After that, frames go through unchanged in both directions, binary
//! terminal frames and the server's own requests to the desktop included. Every
//! other HTTP request is forwarded as it is.
//!
//! The exceptions: terminal calls for a session vornd itself holds are
//! answered here and never reach the server ([`crate::terminal`]), and so
//! are the calls of the groups vornd has taken over ([`crate::native`]). What
//! vornd sends a client and what the server sends it share one ordered
//! outbox per connection ([`crate::streams::ClientConn`]). A connection that
//! opens with the desktop's launch token in its `Authorization` is the
//! desktop's, which the size rule favours ([`crate::size`]); nothing a
//! client says later changes that.
//!
//! Headers that describe the client's request go through untouched, `Host` and
//! `Origin` in particular: the server checks that they match, and both name
//! vornd's address, which is what the client connected to. vornd listens only on
//! loopback, so the server judging every peer to be on this machine is true.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, info, warn};

use crate::applink::AppLink;
use crate::groups::{Counted, Groups, Mode};
use crate::holder::Holder;
use crate::native::script::Scripts;
use crate::native::{Conn, Native, Offer, AUTH_GROUP, ORIGIN_METHOD};
use crate::protocol::{
    inspect_server_frame, method_of, ServerFrame, SERVER_PROTOCOLS, VORND_PROTOCOL,
};
use crate::streams::{Forwarder, Streams};

/// The path vornd answers itself. Everything else belongs to the server.
pub const HEALTH_PATH: &str = "/vornd/health";

/// Sent with every accepted WebSocket, so a client can tell it is behind vornd
/// and which version, without anything changing in the frames.
pub const VORND_PROTOCOL_HEADER: HeaderName = HeaderName::from_static("vornd-protocol");

/// The server's own limit for one message is 100 MB; vornd allows a little more
/// so it is never the one to refuse.
const MAX_MESSAGE: usize = 128 << 20;

/// How long the second half of a connection gets to finish once the first ends.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

const UPSTREAM_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) type Body = BoxBody<Bytes, hyper::Error>;

/// Frames from a client waiting to be written to the server.
const UPSTREAM_QUEUE: usize = 64;

/// One vornd: where the server is, the group switches and what it has seen.
pub struct Daemon {
    upstream: SocketAddr,
    groups: Arc<Groups>,
    /// What answers the native groups' calls, when any group is not forwarded.
    native: Option<Arc<Native>>,
    client: Client<HttpConnector, Incoming>,
    probe: Client<HttpConnector, Full<Bytes>>,
    open: AtomicU64,
    served: AtomicU64,
    started: Instant,
    holder: Option<Arc<Holder>>,
    streams: Arc<Streams>,
    /// Whether `vornd:spawn` is answered (`--debug-spawn`).
    spawn: std::sync::atomic::AtomicBool,
    /// The desktop's launch token, when the app that started vornd gave it.
    desktop_token: std::sync::OnceLock<Vec<u8>>,
    /// Where vornd itself listens, which its MCP tools call back to.
    listen: std::sync::OnceLock<SocketAddr>,
    /// The MCP server, made on the first request it may answer.
    mcp: std::sync::OnceLock<Arc<crate::mcp::Mcp>>,
}

impl Daemon {
    pub fn new(upstream: SocketAddr, groups: Groups) -> Arc<Daemon> {
        Daemon::build(upstream, groups, None)
    }

    /// A daemon that also reports on the session holder it keeps.
    pub fn with_holder(upstream: SocketAddr, groups: Groups, holder: Arc<Holder>) -> Arc<Daemon> {
        Daemon::build(upstream, groups, Some(holder))
    }

    fn build(upstream: SocketAddr, groups: Groups, holder: Option<Arc<Holder>>) -> Arc<Daemon> {
        // The engine's streams, so its sessions' clients are served here.
        #[cfg(feature = "engine")]
        let streams = holder
            .as_ref()
            .and_then(|h| h.engine())
            .map(|e| Arc::clone(e.streams()))
            .unwrap_or_default();
        #[cfg(not(feature = "engine"))]
        let streams = Streams::new();
        let native = groups.any_native().then(|| {
            let native = Native::new();
            native.prepare();
            // The session reads answer from the engine's copy of the
            // server's records, which the server feeds only when asked.
            #[cfg(feature = "engine")]
            if let Some(engine) = holder.as_ref().and_then(|h| h.engine()) {
                native.set_registry(Arc::clone(engine.registry()));
                // The copy decides every terminal's status; the server takes them from it.
                engine.decide_statuses();
                // The terminals vornd creates are started in the engine.
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    let host = crate::engine::EngineHost::new(Arc::clone(engine), runtime);
                    native.set_host(Arc::new(host));
                }
            }
            native
        });
        Arc::new(Daemon {
            native,
            streams,
            spawn: std::sync::atomic::AtomicBool::new(false),
            desktop_token: std::sync::OnceLock::new(),
            listen: std::sync::OnceLock::new(),
            mcp: std::sync::OnceLock::new(),
            upstream,
            groups: Arc::new(groups),
            client: Client::builder(TokioExecutor::new()).build_http(),
            probe: Client::builder(TokioExecutor::new()).build_http(),
            open: AtomicU64::new(0),
            served: AtomicU64::new(0),
            started: Instant::now(),
            holder,
        })
    }

    pub fn groups(&self) -> &Groups {
        &self.groups
    }

    /// The server's database, `vorn.db`, which the native groups read to tell
    /// a project on this machine from one on a remote host. Without it, every
    /// call naming a project goes to the server.
    pub fn set_database(&self, db: std::path::PathBuf) {
        if let Some(native) = &self.native {
            native.set_database(db.clone());
        }
        // The scheduler is shadowed by watching the server fire its schedules.
        if self.groups.mode("scheduler") == Mode::Shadow {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let locks = crate::native::work::lock_dir();
                let groups = Arc::clone(&self.groups);
                runtime.spawn(crate::native::work::watch(db, locks, groups));
            }
        }
    }

    /// Answers `vornd:spawn` from now on: sessions started in sessiond
    /// through vornd, for tests until the app creates sessions this way.
    pub fn allow_spawn(&self) {
        self.spawn.store(true, Ordering::Relaxed);
    }

    /// The desktop's launch token: a WebSocket that opens with it as its
    /// bearer credential is the desktop's. Only the first one given is kept.
    pub fn set_desktop_token(&self, token: Vec<u8>) {
        if !token.is_empty() {
            if let Some(native) = &self.native {
                native.set_desktop_token(token.clone());
            }
            let _ = self.desktop_token.set(token);
        }
    }

    /// The app's channel, for what the native groups ask of the server.
    /// Also reads, now and whenever the server says where it is bound,
    /// which names a browser may load the web client from.
    pub fn set_app_link(&self, link: Arc<AppLink>) {
        let Some(native) = &self.native else {
            return;
        };
        native.set_link(Arc::clone(&link));
        native.set_server_port(self.upstream.port());
        if self.groups.mode("terminal") == Mode::Native {
            link.set_creates_terminals();
        }
        if self.groups.mode("headless") == Mode::Native {
            link.set_creates_headless();
        }
        match self.groups.mode("script") {
            Mode::Forward => {}
            mode => link.set_scripts(Scripts::new(mode, native, Arc::clone(&self.groups))),
        }
        // Checked once the login shell's environment is in, so git is on its PATH.
        if link.restores() {
            let n = Arc::clone(native);
            tokio::task::spawn_blocking(move || crate::native::sessions::verify_restored(&n));
        }
        let native = Arc::clone(native);
        tokio::spawn(async move {
            loop {
                let n = Arc::clone(&native);
                let _ = tokio::task::spawn_blocking(move || n.refresh_trusted()).await;
                link.reached().await;
            }
        });
    }

    /// Where vornd listens. Its MCP tools reach the server through it, so
    /// their calls are routed as any client's are.
    pub fn set_listen_addr(&self, addr: SocketAddr) {
        let _ = self.listen.set(addr);
    }

    /// Whether a WebSocket that opened with these headers is the desktop's.
    fn is_desktop(&self, headers: &HeaderMap) -> bool {
        let Some(token) = self.desktop_token.get() else {
            return false;
        };
        let auth = headers
            .get(header::AUTHORIZATION)
            .map(HeaderValue::as_bytes);
        is_desktop_credential(auth, token)
    }

    /// Asks the server's own health route, and answers its status if it answered.
    pub async fn probe_upstream(&self) -> Option<StatusCode> {
        let uri: Uri = format!("http://{}/health", self.upstream).parse().ok()?;
        let req = Request::get(uri).body(Full::new(Bytes::new())).ok()?;
        match tokio::time::timeout(UPSTREAM_PROBE_TIMEOUT, self.probe.request(req)).await {
            Ok(Ok(res)) => Some(res.status()),
            _ => None,
        }
    }
}

/// Accepts connections until `shutdown` resolves.
pub async fn serve(
    listener: TcpListener,
    daemon: Arc<Daemon>,
    shutdown: impl std::future::Future<Output = ()>,
) {
    tokio::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(pair) => pair,
                Err(err) => {
                    warn!(%err, "accept failed");
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let daemon = daemon.clone();
        tokio::spawn(async move {
            let service = service_fn(move |req| handle(daemon.clone(), req, peer));
            let conn = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades();
            if let Err(err) = conn.await {
                debug!(%peer, %err, "connection ended with an error");
            }
        });
    }
}

async fn handle(
    daemon: Arc<Daemon>,
    req: Request<Incoming>,
    peer: SocketAddr,
) -> Result<Response<Body>, Infallible> {
    #[cfg(feature = "engine")]
    if req.uri().path() == crate::engine::SESSIONS_PATH && req.method() == Method::GET {
        let digests = req
            .uri()
            .query()
            .is_some_and(|q| q.split('&').any(|kv| kv == "digest=1"));
        return Ok(sessions(&daemon, digests).await);
    }
    if req.uri().path() == HEALTH_PATH && req.method() == Method::GET {
        return Ok(health(&daemon).await);
    }
    if is_websocket_upgrade(req.headers()) {
        return Ok(websocket(daemon, req).await);
    }
    if req.method() == Method::POST && crate::pair::is_pair_path(req.uri().path()) {
        if let Some(native) = daemon
            .native
            .as_ref()
            .filter(|_| daemon.groups.mode("pairing") == Mode::Native)
        {
            let native = Arc::clone(native);
            return Ok(crate::pair::answer(&daemon, &native, req, peer).await);
        }
    }
    if req.uri().path() == crate::mcp::PATH {
        match daemon.groups.mode(crate::mcp::GROUP) {
            Mode::Native => return Ok(mcp(&daemon, req).await),
            // A relay talks to the TypeScript tools or to these, never both,
            // so there is nothing to compare.
            Mode::Shadow => {
                daemon
                    .groups
                    .count(crate::mcp::COUNTED_AS, Counted::Forwarded);
                daemon
                    .groups
                    .count(crate::mcp::COUNTED_AS, Counted::ShadowUnported);
            }
            Mode::Forward => daemon
                .groups
                .count(crate::mcp::COUNTED_AS, Counted::Forwarded),
        }
    }
    Ok(forward_http(&daemon, req).await)
}

/// `/mcp` answered here ([`crate::mcp`]), for a caller it lets in.
async fn mcp(daemon: &Daemon, req: Request<Incoming>) -> Response<Body> {
    let token = daemon.desktop_token.get().map(Vec::as_slice);
    if let Some(refused) = crate::mcp::refusal(req.headers(), token) {
        return refused;
    }
    let (Some(token), Some(addr)) = (token, daemon.listen.get()) else {
        return plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "vornd is not ready to serve MCP",
        );
    };
    let mcp = daemon
        .mcp
        .get_or_init(|| Arc::new(crate::mcp::Mcp::new(*addr, token)));
    crate::mcp::answer(&daemon.groups, mcp, req).await
}

async fn health(daemon: &Daemon) -> Response<Body> {
    let upstream = daemon.probe_upstream().await;
    let reachable = upstream.is_some_and(|s| s.is_success());
    let counts = daemon.groups.counts();
    let mut groups = serde_json::Map::new();
    for (group, mode) in daemon.groups.modes() {
        groups.insert(group.to_string(), json!({ "mode": mode.name() }));
    }
    for (group, c) in &counts {
        let entry = groups
            .entry(group.clone())
            .or_insert_with(|| json!({ "mode": daemon.groups.mode(group).name() }));
        entry["forwarded"] = json!(c.forwarded);
        entry["native"] = json!(c.native);
        entry["shadowMatched"] = json!(c.shadow_matched);
        entry["shadowMismatched"] = json!(c.shadow_mismatched);
        entry["shadowUnported"] = json!(c.shadow_unported);
    }
    if let (Some(mcp), Some(entry)) = (daemon.mcp.get(), groups.get_mut(crate::mcp::GROUP)) {
        entry["sessions"] = json!(mcp.sessions());
    }
    let body = json!({
        "ok": reachable,
        "protocol": VORND_PROTOCOL,
        "serverProtocols": [SERVER_PROTOCOLS.start(), SERVER_PROTOCOLS.end()],
        "upstream": {
            "address": daemon.upstream.to_string(),
            "reachable": reachable,
            "status": upstream.map(|s| s.as_u16()),
        },
        "connections": {
            "open": daemon.open.load(Ordering::Relaxed),
            "served": daemon.served.load(Ordering::Relaxed),
        },
        "uptimeSeconds": daemon.started.elapsed().as_secs(),
        "groups": groups,
        "sessiond": daemon.holder.as_ref().map(|h| h.report()),
        "registry": registry_report(daemon),
    });
    let status = if reachable {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let mut res = Response::new(full(body.to_string()));
    *res.status_mut() = status;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// Where the copy of the server's session records stands, when vornd keeps
/// one.
#[cfg(feature = "engine")]
fn registry_report(daemon: &Daemon) -> Option<serde_json::Value> {
    let registry = daemon.holder.as_ref()?.engine()?.registry();
    registry.wanted().then(|| registry.report())
}

#[cfg(not(feature = "engine"))]
fn registry_report(_: &Daemon) -> Option<serde_json::Value> {
    None
}

/// The session engine's report: where each session is and how it was
/// recovered, and with `digests` (`?digest=1`) each running session's
/// state digest. Never the screen, title or cwd; a digest is a hash of the
/// whole state and gives none of them away. 404 when vornd keeps no holder
/// with an engine.
#[cfg(feature = "engine")]
async fn sessions(daemon: &Daemon, digests: bool) -> Response<Body> {
    let Some(engine) = daemon.holder.as_ref().and_then(|h| h.engine()) else {
        return plain(StatusCode::NOT_FOUND, "no session engine");
    };
    let report = if digests {
        engine.report_with_digests().await
    } else {
        engine.report()
    };
    let mut res = Response::new(full(report.to_string()));
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

async fn forward_http(daemon: &Daemon, req: Request<Incoming>) -> Response<Body> {
    let (mut parts, body) = req.into_parts();
    let path = parts.uri.path_and_query().map_or("/", |p| p.as_str());
    parts.uri = match format!("http://{}{}", daemon.upstream, path).parse() {
        Ok(uri) => uri,
        Err(_) => return plain(StatusCode::BAD_REQUEST, "bad request path"),
    };
    strip_hop_by_hop(&mut parts.headers);
    match daemon
        .client
        .request(Request::from_parts(parts, body))
        .await
    {
        Ok(res) => {
            let (mut parts, body) = res.into_parts();
            strip_hop_by_hop(&mut parts.headers);
            Response::from_parts(parts, body.boxed())
        }
        Err(err) => {
            warn!(%err, "the server did not answer an HTTP request");
            plain(StatusCode::BAD_GATEWAY, "the Vorn server is not answering")
        }
    }
}

impl Daemon {
    /// Hands the server a request whose body vornd has already read.
    pub(crate) async fn forward_buffered(
        &self,
        mut parts: hyper::http::request::Parts,
        body: Bytes,
    ) -> Response<Body> {
        let path = parts.uri.path_and_query().map_or("/", |p| p.as_str());
        parts.uri = match format!("http://{}{}", self.upstream, path).parse() {
            Ok(uri) => uri,
            Err(_) => return plain(StatusCode::BAD_REQUEST, "bad request path"),
        };
        strip_hop_by_hop(&mut parts.headers);
        match self
            .probe
            .request(Request::from_parts(parts, Full::new(body)))
            .await
        {
            Ok(res) => {
                let (mut parts, body) = res.into_parts();
                strip_hop_by_hop(&mut parts.headers);
                Response::from_parts(parts, body.boxed())
            }
            Err(err) => {
                warn!(%err, "the server did not answer an HTTP request");
                plain(StatusCode::BAD_GATEWAY, "the Vorn server is not answering")
            }
        }
    }
}

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

async fn websocket(daemon: Arc<Daemon>, mut req: Request<Incoming>) -> Response<Body> {
    let Some(key) = req.headers().get(header::SEC_WEBSOCKET_KEY).cloned() else {
        return plain(StatusCode::BAD_REQUEST, "missing Sec-WebSocket-Key");
    };
    let desktop = daemon.is_desktop(req.headers());
    // The Origin check, which the server makes on `/ws` alone.
    let origin = daemon
        .native
        .as_ref()
        .filter(|_| req.uri().path() == "/ws")
        .and_then(|native| {
            let mode = daemon.groups.mode(AUTH_GROUP);
            (mode != Mode::Forward).then(|| (mode, origin_allowed(native, req.headers())))
        });
    if let Some((Mode::Native, false)) = origin {
        daemon.groups.count(ORIGIN_METHOD, Counted::Native);
        return origin_refused();
    }
    let credential = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(vorn_reach::token::bearer_from)
        .map(str::to_owned);
    let path = req.uri().path_and_query().map_or("/", |p| p.as_str());
    let mut upstream_req = match format!("ws://{}{}", daemon.upstream, path).into_client_request() {
        Ok(r) => r,
        Err(_) => return plain(StatusCode::BAD_REQUEST, "bad request path"),
    };
    for (name, value) in req.headers() {
        if is_hop_by_hop(name) || is_handshake_header(name) {
            continue;
        }
        if name == header::HOST {
            upstream_req
                .headers_mut()
                .insert(header::HOST, value.clone());
        } else {
            upstream_req
                .headers_mut()
                .append(name.clone(), value.clone());
        }
    }

    let (server, accepted) =
        match tokio_tungstenite::connect_async_with_config(upstream_req, Some(ws_config()), false)
            .await
        {
            Ok(pair) => {
                count_origin(&daemon.groups, origin, true);
                pair
            }
            Err(tungstenite::Error::Http(refused)) => {
                if refused.status() == StatusCode::FORBIDDEN {
                    count_origin(&daemon.groups, origin, false);
                }
                // The server said no. The client hears the same answer.
                let (mut parts, body) = refused.into_parts();
                strip_hop_by_hop(&mut parts.headers);
                return Response::from_parts(parts, full(body.unwrap_or_default()));
            }
            Err(err) => {
                warn!(%err, "could not open a WebSocket to the server");
                return plain(StatusCode::BAD_GATEWAY, "the Vorn server is not answering");
            }
        };

    let on_upgrade = hyper::upgrade::on(&mut req);
    let mut res = Response::new(full(Bytes::new()));
    *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    let headers = res.headers_mut();
    headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
    if let Ok(accept) = HeaderValue::from_str(&derive_accept_key(key.as_bytes())) {
        headers.insert(header::SEC_WEBSOCKET_ACCEPT, accept);
    }
    headers.insert(VORND_PROTOCOL_HEADER, HeaderValue::from(VORND_PROTOCOL));
    if let Some(protocol) = accepted.headers().get(header::SEC_WEBSOCKET_PROTOCOL) {
        headers.insert(header::SEC_WEBSOCKET_PROTOCOL, protocol.clone());
    }

    tokio::spawn(async move {
        let upgraded = match on_upgrade.await {
            Ok(u) => u,
            Err(err) => {
                debug!(%err, "the client left before the upgrade finished");
                return;
            }
        };
        let client = WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            Role::Server,
            Some(ws_config()),
        )
        .await;
        daemon.open.fetch_add(1, Ordering::Relaxed);
        daemon.served.fetch_add(1, Ordering::Relaxed);
        pump(&daemon, client, server, desktop, credential).await;
        daemon.open.fetch_sub(1, Ordering::Relaxed);
    });
    res
}

/// Moves frames both ways until either side closes, then gives the other side a
/// moment to finish its close.
///
/// Frames to the client go through the connection's outbox, which one writer
/// drains, so the server's frames and what vornd answers itself stay in the
/// order they were queued.
async fn pump<C, S>(
    daemon: &Arc<Daemon>,
    client: WebSocketStream<C>,
    server: WebSocketStream<S>,
    desktop: bool,
    credential: Option<String>,
) where
    C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut to_client, mut from_client) = client.split();
    let (mut to_server, mut from_server) = server.split();
    let mut conn = daemon.streams.connect();
    let conn_id = conn.id();
    if desktop {
        mark_desktop(daemon, conn_id);
    }
    let forward = conn.forwarder();

    let writer = tokio::spawn(async move {
        while let Some(o) = conn.next().await {
            let size = o.size();
            let closing = matches!(o.msg, Message::Close(_));
            if to_client.send(o.msg).await.is_err() {
                break;
            }
            conn.written(size);
            if closing {
                break;
            }
        }
        let _ = to_client.close().await;
    });

    // Everything for the server goes through one queue, so a call vornd
    // took and then found to be the server's joins the client's other frames
    // there. The writer closes the server's side once the client's frames
    // have stopped and no such call is still running.
    let (up_tx, mut up_rx) = tokio::sync::mpsc::channel::<Message>(UPSTREAM_QUEUE);
    tokio::spawn(async move {
        while let Some(msg) = up_rx.recv().await {
            if to_server.send(msg).await.is_err() {
                break;
            }
        }
        let _ = to_server.close().await;
    });
    let native = daemon.native.as_ref().map(|n| {
        Conn::new(
            Arc::clone(n),
            Arc::clone(&daemon.groups),
            forward.clone(),
            &up_tx,
            desktop,
        )
    });
    if let (Some(native), Some(credential)) = (&native, credential) {
        native.check_credential(credential);
    }

    let groups_daemon = daemon.clone();
    let reply = forward.clone();
    let offered = native.clone();
    let upward = tokio::spawn(async move {
        while let Some(Ok(msg)) = from_client.next().await {
            match &msg {
                Message::Text(text) => {
                    if answered_here(&groups_daemon, conn_id, &reply, text.as_str()) {
                        continue;
                    }
                    if let Some(method) = method_of(text.as_str()) {
                        match &offered {
                            Some(native) => {
                                if native.offer(&method, text.as_str()) == Offer::Taken {
                                    continue;
                                }
                            }
                            None => groups_daemon
                                .groups()
                                .count(&method, crate::groups::Counted::Forwarded),
                        }
                    }
                }
                // Each side answers its own pings.
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                _ => {}
            }
            if up_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    let streams = Arc::clone(&daemon.streams);
    let downward = tokio::spawn(async move {
        while let Some(Ok(msg)) = from_server.next().await {
            let msg = match msg {
                Message::Text(text) if told_here(&streams, text.as_str()) => continue,
                Message::Text(text) => {
                    if let Some(native) = &native {
                        native.on_server_text(text.as_str());
                    }
                    match inspect_server_frame(text.as_str()) {
                        ServerFrame::Pass => Message::Text(text),
                        ServerFrame::Unsupported(version) => {
                            warn!(
                                ?version,
                                "the server speaks a protocol vornd does not know; closing"
                            );
                            forward
                                .send(Message::Close(Some(CloseFrame {
                                    code: CloseCode::Error,
                                    reason: "vornd does not support this server's protocol version"
                                        .into(),
                                })))
                                .await;
                            return;
                        }
                    }
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                Message::Close(frame) => {
                    if let (Some(native), Some(f)) = (&native, &frame) {
                        native.on_server_close(u16::from(f.code));
                    }
                    Message::Close(frame)
                }
                other => other,
            };
            // The writer sends a close after everything queued before it,
            // and then closes the client's side.
            let closing = matches!(msg, Message::Close(_));
            forward.send(msg).await;
            if closing {
                return;
            }
        }
        forward.send(Message::Close(None)).await;
    });

    tokio::pin!(upward, downward, writer);
    tokio::select! {
        _ = &mut upward => {
            if tokio::time::timeout(CLOSE_GRACE, &mut downward).await.is_err() {
                downward.abort();
            }
        }
        _ = &mut downward => {
            if tokio::time::timeout(CLOSE_GRACE, &mut upward).await.is_err() {
                upward.abort();
            }
        }
    }
    if tokio::time::timeout(CLOSE_GRACE, &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
}

/// Whether the upgrade's `Origin` is one the server allows.
fn origin_allowed(native: &Native, headers: &HeaderMap) -> bool {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let origin = match (origins.next(), origins.next()) {
        (None, _) => None,
        // The server sees two as one, joined with a comma, which no URL is.
        (Some(_), Some(_)) => return false,
        (Some(v), None) => match v.to_str() {
            Ok(s) => Some(s),
            Err(_) => return false,
        },
    };
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    vorn_reach::origin::is_allowed_upgrade(origin, host, &native.trusted())
}

/// What the server answers an upgrade from a page it does not trust.
fn origin_refused() -> Response<Body> {
    let mut res = Response::new(full(r#"{"error":"Origin not allowed"}"#));
    *res.status_mut() = StatusCode::FORBIDDEN;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    res
}

/// Counts the server's answer to the Origin check against vornd's, in
/// shadow mode.
fn count_origin(groups: &Groups, origin: Option<(Mode, bool)>, server: bool) {
    match origin {
        Some((Mode::Shadow, ours)) if ours == server => {
            groups.count(ORIGIN_METHOD, Counted::ShadowMatched)
        }
        Some((Mode::Shadow, _)) => {
            warn!(server, "the Origin check differs from the server's");
            groups.count(ORIGIN_METHOD, Counted::ShadowMismatched)
        }
        Some((Mode::Native, _)) => groups.count(ORIGIN_METHOD, Counted::Native),
        _ => {}
    }
}

/// Whether a frame from the server is a session's exit that vornd tells
/// clients about itself ([`Streams::answers_for`]).
fn told_here(streams: &Streams, text: &str) -> bool {
    if !text.contains("\"terminal:exit\"") {
        return false;
    }
    let Ok(serde_json::Value::Object(frame)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return false;
    };
    frame.get("method").and_then(|m| m.as_str()) == Some("terminal:exit")
        && frame
            .get("params")
            .and_then(|p| p.get("id"))
            .and_then(|i| i.as_str())
            .is_some_and(|id| streams.answers_for(id))
}

/// Whether vornd answered a client's frame itself.
#[cfg(feature = "engine")]
fn answered_here(daemon: &Daemon, conn: u64, reply: &Forwarder, text: &str) -> bool {
    let Some(engine) = daemon.holder.as_ref().and_then(|h| h.engine()) else {
        return false;
    };
    let spawn = daemon.spawn.load(Ordering::Relaxed);
    crate::terminal::handle(engine, conn, reply, text, spawn)
}

#[cfg(not(feature = "engine"))]
fn answered_here(_: &Daemon, _: u64, _: &Forwarder, _: &str) -> bool {
    false
}

/// Tells the size rule that connection `conn` is the desktop's.
#[cfg(feature = "engine")]
fn mark_desktop(daemon: &Daemon, conn: u64) {
    if let Some(engine) = daemon.holder.as_ref().and_then(|h| h.engine()) {
        engine.sizes().desktop(conn);
    }
}

#[cfg(not(feature = "engine"))]
fn mark_desktop(_: &Daemon, _: u64) {}

/// Whether the `Authorization` a WebSocket opened with carries `token`,
/// compared in constant time: the desktop's launch token decides who is a
/// desktop, and the comparison must not say how much of a guess was right.
pub fn is_desktop_credential(authorization: Option<&[u8]>, token: &[u8]) -> bool {
    let Some(presented) = authorization.and_then(|a| a.strip_prefix(b"Bearer ")) else {
        return false;
    };
    let presented = presented.trim_ascii();
    if token.is_empty() || presented.len() != token.len() {
        return false;
    }
    presented
        .iter()
        .zip(token)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// Headers that describe one hop, not the request, and are never forwarded.
fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// The parts of a WebSocket handshake each hop makes for itself. Extensions are
/// dropped so neither hop negotiates compression the other did not agree to.
fn is_handshake_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-extensions"
            | "sec-websocket-accept"
    )
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    // Anything `Connection` names is hop-by-hop too.
    let named: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|n| HeaderName::from_bytes(n.trim().as_bytes()).ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    let fixed: Vec<HeaderName> = headers
        .keys()
        .filter(|n| is_hop_by_hop(n))
        .cloned()
        .collect();
    for name in fixed {
        headers.remove(name);
    }
}

pub(crate) fn full(body: impl Into<Bytes>) -> Body {
    Full::new(body.into())
        .map_err(|never| match never {})
        .boxed()
}

fn plain(status: StatusCode, message: &'static str) -> Response<Body> {
    let mut res = Response::new(full(message));
    *res.status_mut() = status;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    res
}

/// Logs once whether the server answers, for the start of the log.
pub async fn log_upstream(daemon: &Daemon) {
    match daemon.probe_upstream().await {
        Some(status) if status.is_success() => {
            info!(upstream = %daemon.upstream, "the server answers")
        }
        Some(status) => {
            warn!(upstream = %daemon.upstream, %status, "the server answered its health check with an error")
        }
        None => warn!(upstream = %daemon.upstream, "the server is not answering yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_hop_by_hop_headers_and_what_connection_names() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-private"),
        );
        headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        headers.insert("x-private", HeaderValue::from_static("1"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:1"),
        );
        strip_hop_by_hop(&mut headers);
        assert_eq!(headers.len(), 1);
        assert!(headers.contains_key(header::ORIGIN));
    }

    #[test]
    fn knows_a_websocket_upgrade() {
        let mut headers = HeaderMap::new();
        assert!(!is_websocket_upgrade(&headers));
        headers.insert(header::UPGRADE, HeaderValue::from_static("WebSocket"));
        assert!(is_websocket_upgrade(&headers));
    }
}

//! The endpoint: HTTP and WebSocket for every client, one ordered outbox per connection ([`crate::streams::ClientConn`]).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, warn};

use crate::applink::AppLink;
use crate::holder::Holder;
use crate::native::script::Scripts;
use crate::native::Native;
use crate::protocol::VORND_PROTOCOL;
use crate::streams::{Forwarder, Streams};

/// vornd's own report on itself.
pub const HEALTH_PATH: &str = "/vornd/health";

/// The server's local credential, beside the database.
const LOCAL_TOKEN_FILE: &str = "local-token";

/// Sent with every accepted WebSocket, so a client can tell which vornd it reached.
pub const VORND_PROTOCOL_HEADER: HeaderName = HeaderName::from_static("vornd-protocol");

/// The limit for one message, a little over the clients' own 100 MB.
const MAX_MESSAGE: usize = 128 << 20;

pub(crate) type Body = BoxBody<Bytes, hyper::Error>;

/// One vornd: what it serves and what it has seen.
pub struct Daemon {
    /// What vornd keeps as the server, once it serves.
    serving: std::sync::OnceLock<Arc<crate::serve::Serving>>,
    /// What answers the calls.
    native: Arc<Native>,
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
    /// A daemon with the session holder it keeps, if it keeps one.
    pub fn new(holder: Option<Arc<Holder>>) -> Arc<Daemon> {
        // The engine's streams, so its sessions' clients are served here.
        #[cfg(feature = "engine")]
        let streams = holder
            .as_ref()
            .and_then(|h| h.engine())
            .map(|e| Arc::clone(e.streams()))
            .unwrap_or_default();
        #[cfg(not(feature = "engine"))]
        let streams = Streams::new();
        let native = {
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
                    let host = crate::engine::EngineHost::new(Arc::clone(engine), runtime.clone());
                    native.set_host(Arc::new(host));
                    // Clients are told of the sessions from what they do and what the registry records.
                    runtime.spawn(crate::native::session_events::follow(
                        Arc::downgrade(&native),
                        Arc::clone(engine),
                    ));
                }
            }
            native
        };
        Arc::new(Daemon {
            native,
            streams,
            spawn: std::sync::atomic::AtomicBool::new(false),
            desktop_token: std::sync::OnceLock::new(),
            listen: std::sync::OnceLock::new(),
            mcp: std::sync::OnceLock::new(),
            serving: std::sync::OnceLock::new(),
            open: AtomicU64::new(0),
            served: AtomicU64::new(0),
            started: Instant::now(),
            holder,
        })
    }

    /// The server's database, `vorn.db`.
    pub fn set_database(&self, db: std::path::PathBuf) {
        self.native.set_database(db);
    }

    /// Starts the work model ([`crate::native::work`]) once vornd has a
    /// database, its own address and a credential for it: the desktop's, or
    /// the local one published beside the database.
    pub fn start_work(&self) {
        use crate::native::work::{db::Db, host::TOPICS, Work};
        let native = &self.native;
        let (Some(db_path), Some(addr)) = (
            native.database().map(std::path::Path::to_path_buf),
            self.listen.get().copied(),
        ) else {
            return;
        };
        let token = self.desktop_token.get().cloned().or_else(|| {
            let file = db_path.parent()?.join(LOCAL_TOKEN_FILE);
            std::fs::read(file).ok().map(|t| t.trim_ascii().to_vec())
        });
        let Some(token) = token.filter(|t| !t.is_empty()) else {
            warn!("the work model has no credential to reach vornd's endpoint");
            return;
        };
        let Some(db) = Db::open(&db_path) else {
            return;
        };
        let loopback = Arc::new(crate::mcp::Loopback::with_topics(addr, &token, TOPICS));
        let work = Work::new(native, db, loopback);
        native.set_work(Arc::clone(&work));
        work.start();
    }

    /// Starts the endpoint agents' hooks post to ([`crate::native::hooks`]).
    pub async fn start_hooks(&self) {
        self.native.start_hooks().await;
    }

    /// Gives the hook registration up as vornd stops.
    pub fn stop_hooks(&self) {
        self.native.stop_hooks();
    }

    /// Tells the status widget's list as the sessions change ([`crate::native::widget`]).
    pub fn start_widget(&self) {
        self.native.start_widget();
    }

    /// Starts connections and connectors ([`crate::native::connectors`]) once
    /// vornd has a database and its own address, which a browser connector's
    /// child reaches its window through.
    pub async fn start_connectors(&self) {
        let native = &self.native;
        let (Some(dir), Some(mut addr)) = (
            native
                .database()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf),
            self.listen.get().copied(),
        ) else {
            return;
        };
        if addr.ip().is_unspecified() {
            addr.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
        }
        let bridge: Arc<dyn crate::bridge::Bridge> = Arc::clone(native.main_process()) as _;
        let connectors = crate::native::connectors::Connectors::new(
            native,
            &dir,
            format!("http://{addr}"),
            bridge,
        );
        native.set_connectors(Arc::clone(&connectors));
        tokio::spawn(async move { connectors.reconcile().await });
    }

    /// Starts the extension host ([`crate::native::extensions`]) once vornd
    /// has a database, its own address and a credential for its endpoint,
    /// which it reads the extensions' terminals through.
    pub async fn start_extensions(self: &Arc<Self>) {
        use crate::native::extensions::{routes, Extensions, Wired};
        use vorn_extensions::host::{HostSettings, Supervisor};
        use vorn_extensions::{pack::PackStore, page};
        let native = &self.native;
        let (Some(db_dir), Some(mut addr)) = (
            native
                .database()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf),
            self.listen.get().copied(),
        ) else {
            return;
        };
        if addr.ip().is_unspecified() {
            addr.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
        }
        let token = self.desktop_token.get().cloned().or_else(|| {
            std::fs::read(db_dir.join(LOCAL_TOKEN_FILE))
                .ok()
                .map(|t| t.trim_ascii().to_vec())
        });
        let Some(token) = token.filter(|t| !t.is_empty()) else {
            warn!("the extension host has no credential to reach vornd's endpoint");
            return;
        };
        let bridge_origin = format!("http://{addr}");
        let settings = HostSettings {
            program: "node".into(),
            base_env: native.child_env(),
            bridge_origin: bridge_origin.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let supervisor = Supervisor::new(PackStore::new(db_dir.join("connectors")), settings);
        let daemon = Arc::downgrade(self);
        let around = Wired {
            native: Arc::downgrade(native),
            loopback: Arc::new(crate::mcp::Loopback::new(addr, &token)),
            clients: Box::new(move || {
                daemon
                    .upgrade()
                    .map_or(0, |d| d.open.load(Ordering::Relaxed))
            }),
        };
        let declared = std::env::var("VORN_APP_ORIGINS").ok();
        let ancestors =
            page::frame_ancestors(&[addr.port(), self.server_port()], declared.as_deref());
        let home = crate::native::shell::home_dir().into();
        let extensions =
            Extensions::new(supervisor, Arc::new(around), bridge_origin, ancestors, home);
        if let Err(err) = routes::start_pages(&extensions).await {
            warn!(%err, "pane pages have no origin; panes will not open");
        }
        native.set_extensions(Arc::clone(&extensions));
        extensions.start();
    }

    /// Answers `vornd:spawn` from clients from now on, for tests.
    pub fn allow_spawn(&self) {
        self.spawn.store(true, Ordering::Relaxed);
    }

    /// The desktop's launch token: a WebSocket that opens with it as its
    /// bearer credential is the desktop's. Only the first one given is kept.
    pub fn set_desktop_token(&self, token: Vec<u8>) {
        if !token.is_empty() {
            self.native.set_desktop_token(token.clone());
            let _ = self.desktop_token.set(token);
        }
    }

    /// What the calls share beyond one connection ([`AppLink`]), and the names a browser may load the web client from.
    pub fn set_app_link(&self, link: Arc<AppLink>) {
        let native = &self.native;
        native.set_link(Arc::clone(&link));
        native.set_server_port(self.server_port());
        link.set_scripts(Scripts::new(native));
        // Checked once the login shell's environment is in, so git is on its PATH.
        let n = Arc::clone(native);
        tokio::task::spawn_blocking(move || crate::native::sessions::verify_restored(&n));
        let native = Arc::clone(native);
        tokio::spawn(async move {
            loop {
                let n = Arc::clone(&native);
                let _ = tokio::task::spawn_blocking(move || n.refresh_trusted()).await;
                link.reached().await;
            }
        });
    }

    /// Where vornd listens, which its MCP tools and work model reach it at.
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

    /// The port clients reach the server on.
    fn server_port(&self) -> u16 {
        match self.serving.get() {
            Some(serving) => serving.addr().port(),
            None => self.listen.get().map_or(0, SocketAddr::port),
        }
    }

    /// vornd is the server from now on, as `serving` keeps it.
    pub fn set_serving(&self, serving: Arc<crate::serve::Serving>) {
        let _ = self.serving.set(serving);
    }

    pub(crate) fn streams(&self) -> &Arc<Streams> {
        &self.streams
    }

    /// What answers the calls.
    pub fn native(&self) -> &Arc<Native> {
        &self.native
    }

    /// Connection `conn` is the desktop's, which the size rule favours.
    pub(crate) fn mark_desktop(&self, conn: u64) {
        mark_desktop(self, conn);
    }

    /// Whether vornd answered a client's terminal frame itself.
    pub(crate) fn answered_here(&self, conn: u64, reply: &Forwarder, text: &str) -> bool {
        answered_here(self, conn, reply, text)
    }
}

/// Serves one accepted connection's requests, upgrades included.
pub(crate) fn serve_connection(
    daemon: &Arc<Daemon>,
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
) {
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

async fn handle(
    daemon: Arc<Daemon>,
    req: Request<Incoming>,
    peer: SocketAddr,
) -> Result<Response<Body>, Infallible> {
    if !peer.ip().is_loopback() && !public_route(req.uri().path()) {
        return Ok(crate::serve::http::not_found());
    }
    let serving = daemon.serving.get().cloned();
    if is_websocket_upgrade(req.headers()) {
        return Ok(match serving {
            Some(serving) => serve_websocket(daemon, serving, req, peer).await,
            None => crate::serve::http::not_found(),
        });
    }
    let path = req.uri().path();
    if let Some(serving) = &serving {
        if req.method() == Method::GET && crate::serve::http::answers(path) {
            return Ok(crate::serve::http::answer(
                path,
                serving.web(),
                serving.data_dir(),
            ));
        }
    }
    #[cfg(feature = "engine")]
    if path == crate::engine::SESSIONS_PATH && req.method() == Method::GET {
        let digests = req
            .uri()
            .query()
            .is_some_and(|q| q.split('&').any(|kv| kv == "digest=1"));
        return Ok(sessions(&daemon, digests).await);
    }
    if path == HEALTH_PATH && req.method() == Method::GET {
        return Ok(health(&daemon));
    }
    if crate::native::work::routes::is_route(path) {
        return Ok(match daemon.native.work() {
            Some(work) => {
                let work = Arc::clone(work);
                crate::native::work::routes::answer(&work, req, peer).await
            }
            None => plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "vornd is not running workflows",
            ),
        });
    }
    if let Some(id) = window_route(req.method(), path) {
        if let Some(connectors) = daemon.native.connectors().cloned() {
            return Ok(window_fetch(&connectors, &id, req, peer).await);
        }
    }
    if let Some(route) = crate::native::extensions::routes::bridge_route(req.method(), path) {
        if let Some(extensions) = daemon.native.extensions() {
            let extensions = Arc::clone(extensions);
            return Ok(
                crate::native::extensions::routes::answer(&extensions, route, req, peer).await,
            );
        }
    }
    if req.method() == Method::POST && crate::pair::is_pair_path(path) {
        let native = Arc::clone(&daemon.native);
        return Ok(crate::pair::answer(&native, req, peer).await);
    }
    if path == crate::mcp::PATH {
        return Ok(mcp(&daemon, req).await);
    }
    Ok(crate::serve::http::not_found())
}

/// The routes a peer on another machine may reach.
fn public_route(path: &str) -> bool {
    path == "/ws"
        || crate::serve::http::answers(path)
        || crate::pair::is_pair_path(path)
        || path.starts_with("/artifact/")
        || path.starts_with("/gate-view/")
}

/// A WebSocket with vornd as the server: only `/ws`, from a page it trusts.
async fn serve_websocket(
    daemon: Arc<Daemon>,
    serving: Arc<crate::serve::Serving>,
    mut req: Request<Incoming>,
    peer: SocketAddr,
) -> Response<Body> {
    if req.uri().path() != "/ws" {
        return crate::serve::http::not_found();
    }
    let Some(key) = req.headers().get(header::SEC_WEBSOCKET_KEY).cloned() else {
        return plain(StatusCode::BAD_REQUEST, "missing Sec-WebSocket-Key");
    };
    if !origin_allowed(&daemon.native, req.headers()) {
        return origin_refused();
    }
    let desktop = daemon.is_desktop(req.headers());
    let credential = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(vorn_reach::token::bearer_from)
        .map(str::to_owned);
    let topics = crate::serve::clients::Topics::from_query(req.uri().query());
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
        crate::serve::socket::run(
            Arc::clone(&daemon),
            serving,
            client,
            desktop,
            credential,
            peer,
            topics,
        )
        .await;
        daemon.open.fetch_sub(1, Ordering::Relaxed);
    });
    res
}

/// The connection a browser connector's child calls its window for:
/// `POST /connections/<id>/browser/fetch`.
fn window_route(method: &Method, path: &str) -> Option<String> {
    let rest = path
        .strip_prefix("/connections/")?
        .strip_suffix("/browser/fetch")?;
    (*method == Method::POST && !rest.is_empty() && !rest.contains('/')).then(|| rest.to_owned())
}

/// A browser connector's child calling through its window, from this machine only.
async fn window_fetch(
    connectors: &Arc<crate::native::connectors::Connectors>,
    id: &str,
    req: Request<Incoming>,
    peer: SocketAddr,
) -> Response<Body> {
    let reply = |status: u16, body: serde_json::Value| {
        let mut res = Response::new(full(body.to_string()));
        *res.status_mut() =
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        res.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        res
    };
    if !peer.ip().is_loopback() {
        return reply(403, json!({ "error": "Local machine only" }));
    }
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(vorn_reach::token::bearer_from)
        .map(str::to_owned);
    let call = req
        .headers()
        .get("x-vorn-session-call")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let limited = http_body_util::Limited::new(req.into_body(), 1024 * 1024 + 1);
    let Ok(body) = limited.collect().await.map(|b| b.to_bytes()) else {
        return reply(413, json!({ "error": "That request is too large" }));
    };
    let (status, answer) = connectors
        .window_fetch(id, bearer.as_deref(), call.as_deref(), &body)
        .await;
    reply(status, answer)
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
    crate::mcp::answer(mcp, req).await
}

fn health(daemon: &Daemon) -> Response<Body> {
    let body = json!({
        "ok": true,
        "protocol": VORND_PROTOCOL,
        "connections": {
            "open": daemon.open.load(Ordering::Relaxed),
            "served": daemon.served.load(Ordering::Relaxed),
        },
        "uptimeSeconds": daemon.started.elapsed().as_secs(),
        "sessiond": daemon.holder.as_ref().map(|h| h.report()),
        "registry": registry_report(daemon),
        "hooks": daemon.native.hooks_activity(),
        "mcp": daemon.mcp.get().map(|mcp| json!({ "sessions": mcp.sessions() })),
        "extensions": daemon.native.extensions().map(|e| json!({ "hosts": e.hosts(), "panes": e.panes() })),
    });
    let mut res = Response::new(full(body.to_string()));
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

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

/// Whether the upgrade's `Origin` is one vornd allows.
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

/// The answer to an upgrade from a page vornd does not trust.
fn origin_refused() -> Response<Body> {
    let mut res = Response::new(full(r#"{"error":"Origin not allowed"}"#));
    *res.status_mut() = StatusCode::FORBIDDEN;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    res
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_a_websocket_upgrade() {
        let mut headers = HeaderMap::new();
        assert!(!is_websocket_upgrade(&headers));
        headers.insert(header::UPGRADE, HeaderValue::from_static("WebSocket"));
        assert!(is_websocket_upgrade(&headers));
    }
}

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
use tracing::{debug, info, warn};

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
            // The session reads answer from the engine's session registry.
            #[cfg(feature = "engine")]
            if let Some(engine) = holder.as_ref().and_then(|h| h.engine()) {
                native.set_adapters(Arc::clone(engine.adapters()));
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
            // Read now: the adapters say which variables no process inherits.
            native.adapters();
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

    /// Relabels installed packages written for the pre-1.0 API, whose
    /// surface 1.0 kept, before anything reads them; once, as each is rewritten.
    pub async fn relabel_installed(&self) {
        let Some(dir) = self.native.database().and_then(std::path::Path::parent) else {
            return;
        };
        let store = vorn_extensions::pack::PackStore::new(dir.join("connectors"));
        let Ok((done, failed)) =
            tokio::task::spawn_blocking(move || store.relabel_installed()).await
        else {
            return;
        };
        for dir in done {
            debug!(dir = %dir.display(), "relabelled for vorn:extension@1.0");
        }
        for why in failed {
            warn!("{why}");
        }
    }

    /// Installs the shipped set ([`crate::shipped`]) through the same path as
    /// any package, offline, before the extension host reads the packages:
    /// on a first launch, and whenever this build carries a newer version.
    /// Says which it installed or updated, for [`Daemon::prepare_shipped`].
    pub async fn install_shipped(&self) -> Vec<String> {
        let Some(dir) = self.native.database().and_then(std::path::Path::parent) else {
            return Vec::new();
        };
        let packages = dir.join("connectors");
        let installs = self.native.installs(dir);
        let record = vorn_extensions::shipped::Record::beside(&packages);
        let Ok(synced) = tokio::task::spawn_blocking(move || {
            vorn_extensions::shipped::sync(&installs, crate::shipped::SET, &record)
        })
        .await
        else {
            return Vec::new();
        };
        if synced.installed.is_empty() && synced.updated.is_empty() {
            return Vec::new();
        }
        debug!(installed = ?synced.installed, updated = ?synced.updated, "the shipped set");
        let adapters = Arc::clone(self.native.adapters());
        let _ = tokio::task::spawn_blocking(move || adapters.load_installed(&packages)).await;
        [synced.installed, synced.updated].concat()
    }

    /// Compiles the components of the shipped packs just installed, in the
    /// background, as an install from Settings does, so no first use waits.
    pub fn prepare_shipped(&self, ids: Vec<String>) {
        let Some(router) = self.native.extensions().map(|e| Arc::clone(e.router())) else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        tokio::spawn(async move {
            for id in ids {
                if let Err(err) = router.prepare_installed(&id).await {
                    warn!("[extensions] {err}");
                }
            }
        });
    }

    /// Starts the work model ([`crate::native::work`]) once vornd has a
    /// database, its own address and a credential for it: the desktop's, or
    /// the local one published beside the database.
    pub fn start_work(&self) {
        use crate::native::work::{db::Db, host::TOPICS, Work};
        let native = &self.native;
        if let Some(dir) = native.database().and_then(std::path::Path::parent) {
            use crate::native::extensions::builtin::{switched_on, tasks::TasksBuiltin};
            // The extension host's registry keeps the switch; until it starts, the file says.
            let on = switched_on(
                &dir.join("builtins.json"),
                crate::native::extensions::builtin::tasks::ID,
                true,
            );
            native.set_tasks(TasksBuiltin::new(native, on));
        }
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

    /// Stops every extension program, pane terminal and connector child as vornd stops,
    /// so none outlives it.
    pub async fn stop_extensions(&self) {
        if let Some(extensions) = self.native.extensions() {
            extensions.stop().await;
        }
        if let Some(connectors) = self.native.connectors() {
            connectors.stop().await;
        }
    }

    /// Starts the Connectors built-in ([`crate::native::extensions::builtin::connectors`]) once
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
        use crate::native::extensions::builtin::{connectors, switched_on};
        let services = connectors::services(native, &dir, format!("http://{addr}")).await;
        // The extension host's registry keeps the switch; until it starts, the file says.
        let on = switched_on(
            &dir.join("builtins.json"),
            vorn_connectors::builtin::ID,
            true,
        );
        let connectors = connectors::ConnectorsBuiltin::new(native, services, on);
        match connectors.inner().move_seeded().await {
            Ok(0) => {}
            Ok(n) => info!(
                moved = n,
                "seeded workflows moved into the Connectors' namespace"
            ),
            Err(err) => warn!(%err, "seeded workflows could not be moved"),
        }
        native.set_connectors(Arc::clone(&connectors));
        tokio::spawn(async move { connectors.reconcile().await });
    }

    /// Starts the extension host ([`crate::native::extensions`]) once vornd
    /// has a database, its own address and a credential for its endpoint,
    /// which it reads the extensions' terminals through.
    pub async fn start_extensions(self: &Arc<Self>) {
        use crate::native::extensions::{routes, Extensions, Wired};
        use vorn_extensions::host::{HostSettings, Hosts};
        use vorn_extensions::router::{Router, RouterSettings};
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
            base_env: native.child_env(),
            bridge_origin: bridge_origin.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        // Connectors and extensions are one kind of package, kept in one store.
        let store = PackStore::new(db_dir.join("connectors"));
        let lifecycle = Arc::clone(native.lifecycle());
        let hosts = Hosts::new(Arc::clone(&lifecycle), store.clone(), settings);
        let world = Arc::new(crate::native::extensions::world::Wired {
            native: Arc::downgrade(native),
        });
        let mut router_settings =
            RouterSettings::new(native.child_env(), env!("CARGO_PKG_VERSION"));
        router_settings.blobs = Some(db_dir.join("extension-files"));
        // Tasks keeps its images where the board always kept them.
        router_settings.placed_blobs = vec![(
            crate::native::extensions::builtin::tasks::ID.to_owned(),
            db_dir.join(crate::native::extensions::builtin::tasks::IMAGES),
        )];
        router_settings.compiler = wasm_compiler();
        router_settings.preferences = Some(db_dir.join("extension-preferences.json"));
        let router = Router::new(
            store.clone(),
            lifecycle,
            world,
            router_settings,
            Arc::clone(native.events()),
        );
        let installs = native.installs(&db_dir);
        let around = Wired {
            native: Arc::downgrade(native),
            loopback: Arc::new(crate::mcp::Loopback::new(addr, &token)),
        };
        let declared = std::env::var("VORN_APP_ORIGINS").ok();
        let ancestors =
            page::frame_ancestors(&[addr.port(), self.server_port()], declared.as_deref());
        let home = crate::native::shell::home_dir().into();
        let extensions = Extensions::new(
            hosts,
            router,
            installs,
            Arc::new(around),
            bridge_origin,
            ancestors,
            home,
        );
        if let Err(err) = routes::start_pages(&extensions).await {
            warn!(%err, "pane pages have no origin; panes will not open");
        }
        native.set_extensions(Arc::clone(&extensions));
        extensions.start();
        crate::native::extensions::builtin::precompile(extensions.router());
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

    /// Whether `/grid` is served: only while vornd keeps the sessions.
    fn serves_grid(&self) -> bool {
        #[cfg(feature = "engine")]
        return self.holder.as_ref().and_then(|h| h.engine()).is_some();
        #[cfg(not(feature = "engine"))]
        false
    }

    /// Serves a `/grid` socket until it closes.
    async fn serve_grid<C>(
        &self,
        serving: Arc<crate::serve::Serving>,
        client: WebSocketStream<C>,
        desktop: bool,
        credential: Option<String>,
        peer: std::net::IpAddr,
    ) where
        C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        #[cfg(feature = "engine")]
        if let Some(engine) = self.holder.as_ref().and_then(|h| h.engine()) {
            crate::serve::grid_ws::run(
                Arc::clone(&self.native),
                serving,
                Arc::clone(engine),
                client,
                desktop,
                credential,
                peer,
            )
            .await;
        }
        #[cfg(not(feature = "engine"))]
        let _ = (serving, client, desktop, credential, peer);
    }
}

/// How long another machine has to finish the TLS handshake.
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The first byte of a TLS handshake record.
const TLS_HANDSHAKE: u8 = 0x16;

/// Serves one accepted connection's requests, upgrades included: over TLS
/// from another machine, and from this one over TLS or not, as it opens.
pub(crate) fn serve_connection(
    daemon: &Arc<Daemon>,
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
) {
    let daemon = daemon.clone();
    tokio::spawn(async move {
        let tls = must_use_tls(peer.ip()) || {
            let mut first = [0u8; 1];
            matches!(stream.peek(&mut first).await, Ok(1) if first[0] == TLS_HANDSHAKE)
        };
        if !tls {
            return serve_http(daemon, stream, peer).await;
        }
        let Some(acceptor) = daemon
            .serving
            .get()
            .and_then(|s| s.tls())
            .map(|t| t.acceptor.clone())
        else {
            debug!(%peer, "refused: no certificate to serve another machine with");
            return;
        };
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
            Ok(Ok(stream)) => serve_http(daemon, stream, peer).await,
            Ok(Err(err)) => debug!(%peer, %err, "refused: the TLS handshake failed"),
            Err(_) => debug!(%peer, "refused: the TLS handshake took too long"),
        }
    });
}

/// Another machine's connection, which is never served in the clear.
fn must_use_tls(peer: std::net::IpAddr) -> bool {
    !peer.to_canonical().is_loopback()
}

async fn serve_http<S>(daemon: Arc<Daemon>, stream: S, peer: SocketAddr)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let service = service_fn(move |req| handle(daemon.clone(), req, peer));
    let conn = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades();
    if let Err(err) = conn.await {
        debug!(%peer, %err, "connection ended with an error");
    }
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
                crate::serve::http::takes_brotli(req.headers()),
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
        return Ok(window_fetch(&daemon.native, &id, req, peer).await);
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
        || path == GRID_PATH
        || crate::serve::http::answers(path)
        || crate::pair::is_pair_path(path)
        || path.starts_with("/artifact/")
        || path.starts_with("/gate-view/")
}

/// Grid mode for browsers (`crate::serve::grid_ws`).
const GRID_PATH: &str = "/grid";

/// A WebSocket with vornd as the server: `/ws`, or `/grid` while vornd
/// keeps the sessions, from a page it trusts.
async fn serve_websocket(
    daemon: Arc<Daemon>,
    serving: Arc<crate::serve::Serving>,
    mut req: Request<Incoming>,
    peer: SocketAddr,
) -> Response<Body> {
    let path = req.uri().path();
    let grid = path == GRID_PATH;
    if path != "/ws" && !(grid && daemon.serves_grid()) {
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
        if grid {
            daemon
                .serve_grid(serving, client, desktop, credential, peer.ip())
                .await;
            daemon.open.fetch_sub(1, Ordering::Relaxed);
            return;
        }
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

/// A program calling through its sign-in window, from this machine only.
async fn window_fetch(
    native: &Arc<crate::native::Native>,
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
    let (status, answer) =
        crate::native::signin::window_fetch(native, id, bearer.as_deref(), call.as_deref(), &body)
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

/// Where the session registry stands, when vornd keeps one.
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

/// What compiles a Wasm extension into the cache: `VORN_WASM_COMPILER`, a
/// `vorn` program, else the `vorn` shipped beside vornd. vornd holds no compiler.
fn wasm_compiler() -> vorn_extensions::component::Compiler {
    use vorn_extensions::component::Compiler;
    match std::env::var_os("VORN_WASM_COMPILER").filter(|p| !p.is_empty()) {
        Some(path) => Compiler::Program(path.into()),
        None => Compiler::beside_this_program(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_machine_may_connect_in_the_clear() {
        for other in [
            "192.168.1.20",
            "100.64.0.1",
            "::ffff:192.168.1.20",
            "fe80::1",
            "0.0.0.0",
        ] {
            assert!(must_use_tls(other.parse().unwrap()), "{other}");
        }
        for this in ["127.0.0.1", "::1", "::ffff:127.0.0.1"] {
            assert!(!must_use_tls(this.parse().unwrap()), "{this}");
        }
    }

    #[test]
    fn knows_a_websocket_upgrade() {
        let mut headers = HeaderMap::new();
        assert!(!is_websocket_upgrade(&headers));
        headers.insert(header::UPGRADE, HeaderValue::from_static("WebSocket"));
        assert!(is_websocket_upgrade(&headers));
    }
}

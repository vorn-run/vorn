//! The endpoint: HTTP and WebSocket in front of the Node server.
//!
//! Clients connect to vornd exactly as they would to the server. A WebSocket
//! upgrade is answered only after the server has accepted the same upgrade, so a
//! refusal (a bad Origin, a wrong token) reaches the client as the server gave
//! it. After that, frames go through unchanged in both directions, binary
//! terminal frames and the server's own requests to the desktop included. Every
//! other HTTP request is forwarded as it is.
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

use crate::groups::Groups;
use crate::holder::Holder;
use crate::protocol::{
    inspect_server_frame, method_of, ServerFrame, SERVER_PROTOCOLS, VORND_PROTOCOL,
};

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

type Body = BoxBody<Bytes, hyper::Error>;

/// One vornd: where the server is, the group switches and what it has seen.
pub struct Daemon {
    upstream: SocketAddr,
    groups: Groups,
    client: Client<HttpConnector, Incoming>,
    probe: Client<HttpConnector, Full<Bytes>>,
    open: AtomicU64,
    served: AtomicU64,
    started: Instant,
    holder: Option<Arc<Holder>>,
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
        Arc::new(Daemon {
            upstream,
            groups,
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
            let service = service_fn(move |req| handle(daemon.clone(), req));
            let conn = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades();
            if let Err(err) = conn.await {
                debug!(%peer, %err, "connection ended with an error");
            }
        });
    }
}

async fn handle(daemon: Arc<Daemon>, req: Request<Incoming>) -> Result<Response<Body>, Infallible> {
    #[cfg(feature = "engine")]
    if req.uri().path() == crate::engine::SESSIONS_PATH && req.method() == Method::GET {
        return Ok(sessions(&daemon).await);
    }
    if req.uri().path() == HEALTH_PATH && req.method() == Method::GET {
        return Ok(health(&daemon).await);
    }
    if is_websocket_upgrade(req.headers()) {
        return Ok(websocket(daemon, req).await);
    }
    Ok(forward_http(&daemon, req).await)
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
        entry["shadowUnported"] = json!(c.shadow_unported);
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

/// The session engine's report: where each session is and how it was
/// recovered. 404 when vornd keeps no holder with an engine.
#[cfg(feature = "engine")]
async fn sessions(daemon: &Daemon) -> Response<Body> {
    let Some(engine) = daemon.holder.as_ref().and_then(|h| h.engine()) else {
        return plain(StatusCode::NOT_FOUND, "no session engine");
    };
    let mut res = Response::new(full(engine.report().to_string()));
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

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

async fn websocket(daemon: Arc<Daemon>, mut req: Request<Incoming>) -> Response<Body> {
    let Some(key) = req.headers().get(header::SEC_WEBSOCKET_KEY).cloned() else {
        return plain(StatusCode::BAD_REQUEST, "missing Sec-WebSocket-Key");
    };
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
            Ok(pair) => pair,
            Err(tungstenite::Error::Http(refused)) => {
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
        pump(&daemon, client, server).await;
        daemon.open.fetch_sub(1, Ordering::Relaxed);
    });
    res
}

/// Moves frames both ways until either side closes, then gives the other side a
/// moment to finish its close.
async fn pump<C, S>(daemon: &Arc<Daemon>, client: WebSocketStream<C>, server: WebSocketStream<S>)
where
    C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut to_client, mut from_client) = client.split();
    let (mut to_server, mut from_server) = server.split();

    let groups_daemon = daemon.clone();
    let upward = tokio::spawn(async move {
        while let Some(Ok(msg)) = from_client.next().await {
            match &msg {
                Message::Text(text) => {
                    if let Some(method) = method_of(text.as_str()) {
                        groups_daemon.groups().route(&method);
                    }
                }
                // Each side answers its own pings.
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                _ => {}
            }
            if to_server.send(msg).await.is_err() {
                break;
            }
        }
        let _ = to_server.close().await;
    });

    let downward = tokio::spawn(async move {
        while let Some(Ok(msg)) = from_server.next().await {
            let msg = match msg {
                Message::Text(text) => match inspect_server_frame(text.as_str()) {
                    ServerFrame::Pass => Message::Text(text),
                    ServerFrame::Unsupported(version) => {
                        warn!(
                            ?version,
                            "the server speaks a protocol vornd does not know; closing"
                        );
                        let _ = to_client
                            .send(Message::Close(Some(CloseFrame {
                                code: CloseCode::Error,
                                reason: "vornd does not support this server's protocol version"
                                    .into(),
                            })))
                            .await;
                        break;
                    }
                },
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                other => other,
            };
            if to_client.send(msg).await.is_err() {
                break;
            }
        }
        let _ = to_client.close().await;
    });

    tokio::pin!(upward, downward);
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

fn full(body: impl Into<Bytes>) -> Body {
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

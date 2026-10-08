//! The extension paths over HTTP. On vornd's own port, the bridge an
//! extension's process calls with its token (`POST
//! /extensions/:id/bridge/:method`). On a loopback port of their own, a
//! pane's pages and the bridge those pages call with their nonce: a page
//! shares no storage and no socket with the app, so a hostile script in one
//! holds nothing but that nonce.
//!
//! Every refusal is plain text, which the SDK reads verbatim into the error
//! it raises; any miss on a page is one 404.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Weak};

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::header::{self, HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper::{HeaderMap, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use tracing::{debug, warn};
use vorn_extensions::bridge::{self, Caller, Refusal, Request as Call, Route, SessionView};
use vorn_extensions::pack::InstalledPack;
use vorn_extensions::page;
use vorn_extensions::usage::{usage_for, Conversation};

use super::{Extensions, Live};
use crate::proxy::{full, Body};

/// The largest bridge call taken.
const MAX_BODY: usize = 1 << 20;

/// The bridge route on vornd's own port, if `path` is one.
pub fn bridge_route(method: &Method, path: &str) -> Option<Route> {
    bridge::route(method.as_str(), path).filter(|r| matches!(r, Route::Bridge { .. }))
}

fn refuse(status: u16, message: impl Into<Bytes>) -> Response<Body> {
    let mut res = Response::new(full(message));
    *res.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    res
}

fn refused(refusal: Refusal) -> Response<Body> {
    refuse(refusal.status, refusal.message)
}

fn not_found() -> Response<Body> {
    refuse(404, "Not found")
}

fn header<'a>(headers: &'a HeaderMap, name: HeaderName) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn from_elsewhere(headers: &HeaderMap) -> bool {
    bridge::from_elsewhere(
        header(headers, HeaderName::from_static("sec-fetch-site")),
        header(headers, header::ORIGIN),
        header(headers, header::HOST),
    )
}

/// Answers `route`, which `req` named, for a caller at `peer`.
pub async fn answer<B>(
    extensions: &Extensions,
    route: Route,
    req: Request<B>,
    peer: SocketAddr,
) -> Response<Body>
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    if !peer.ip().is_loopback() {
        return refuse(403, "Local machine only");
    }
    match route {
        Route::Page {
            id,
            pane,
            nonce,
            rest,
        } => serve_page(extensions, &id, &pane, &nonce, &rest).await,
        Route::Bridge { id, method } => {
            if from_elsewhere(req.headers()) {
                return refuse(403, "That request came from another site");
            }
            let caller = bridge::bearer(header(req.headers(), header::AUTHORIZATION))
                .and_then(|token| extensions.supervisor.by_token(&id, token))
                .map(|key| Caller {
                    extension_id: key.extension_id,
                    project_path: key.project_path,
                });
            call(extensions, caller, None, &method, req.into_body()).await
        }
        Route::PaneBridge {
            id,
            pane,
            nonce,
            method,
        } => {
            if from_elsewhere(req.headers()) {
                return refuse(403, "That request came from another site");
            }
            // The session comes from the grant, not the body: a page speaks for the one pane it was opened as.
            let grant = extensions
                .grant(&nonce)
                .filter(|g| g.extension_id == id && g.pane_id == pane);
            let (caller, bound) = grant.map_or((None, None), |g| {
                let caller = Caller {
                    extension_id: g.extension_id,
                    project_path: g.project_path,
                };
                (Some(caller), Some(g.session_id))
            });
            call(extensions, caller, bound, &method, req.into_body()).await
        }
    }
}

/// One bridge call, once the caller is known or known not to be.
async fn call<B>(
    extensions: &Extensions,
    caller: Option<Caller>,
    bound: Option<String>,
    method: &str,
    body: B,
) -> Response<Body>
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let Ok(collected) = Limited::new(body, MAX_BODY).collect().await else {
        return refuse(413, "That call is too large");
    };
    let body = match read_body(&collected.to_bytes()) {
        Some(body) => body,
        None => return refuse(400, "That call is not JSON"),
    };
    let pack = caller
        .as_ref()
        .and_then(|c| extensions.supervisor.store().describe(&c.extension_id));
    if let Err(refusal) = bridge::admit(caller.as_ref(), pack.as_ref(), method) {
        return refused(refusal);
    }
    let Some(caller) = caller else {
        return refuse(401, "This bridge does not know that caller");
    };
    let session =
        bridge::session_named(bound.as_deref(), &body).and_then(|id| extensions.session(id));
    let view = session.as_ref().map(|s| SessionView {
        id: &s.id,
        project_path: &s.project_path,
        renamed_by_person: s.renamed_by_person,
    });
    let asked = match bridge::request(&caller, method, view, &body) {
        Ok(asked) => asked,
        Err(refusal) => return refused(refusal),
    };
    let Some(session) = session else {
        return refuse(404, "That session is not running");
    };
    match run(extensions, asked, &session).await {
        Ok(Some(result)) => {
            let mut res = Response::new(full(json!({ "result": result }).to_string()));
            res.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json; charset=utf-8"),
            );
            res
        }
        Ok(None) => {
            let mut res = Response::new(full(Bytes::new()));
            *res.status_mut() = StatusCode::NO_CONTENT;
            res
        }
        Err(message) => {
            let ext = &caller.extension_id;
            warn!("[extensions] {ext} {method} failed: {message}");
            refuse(500, message)
        }
    }
}

/// A call's body as an object; nothing sent reads as an empty one, and `None` is not JSON.
fn read_body(bytes: &[u8]) -> Option<Map<String, Value>> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Some(Map::new());
    }
    match serde_json::from_slice::<Value>(bytes).ok()? {
        Value::Object(map) => Some(map),
        _ => Some(Map::new()),
    }
}

/// The extension's own reads, once the caller and the session are settled.
async fn run(
    extensions: &Extensions,
    asked: Call,
    session: &Live,
) -> Result<Option<Value>, String> {
    let worktree = session.worktree().to_owned();
    let read_git = |diff: bool| {
        let git = extensions.around.git();
        tokio::task::spawn_blocking(move || {
            let at = Path::new(&worktree);
            let read = if diff {
                git.diff_text(at)
            } else {
                git.status_porcelain(at)
            };
            read.map_err(|e| e.to_string())
        })
    };
    Ok(match asked {
        Call::Diff => Some(Value::String(
            read_git(true).await.map_err(|e| e.to_string())??,
        )),
        Call::Status => Some(Value::String(
            read_git(false).await.map_err(|e| e.to_string())??,
        )),
        Call::Output { lines } => {
            let read = extensions.around.read_output(&session.id, lines).await?;
            Some(Value::String(bridge::output_text(&read)))
        }
        Call::Selection => Some(Value::String(extensions.selection(&session.id).await)),
        Call::Send { text } => {
            extensions.around.write(&session.id, &text).await?;
            None
        }
        Call::Rename { name } => {
            extensions.around.rename(&session.id, &name)?;
            None
        }
        Call::Usage => {
            let of = Conversation {
                agent_session_id: session.agent_session_id.as_deref(),
                worktree: session.worktree_path.as_deref(),
                project: &session.project_path,
            };
            Some(serde_json::to_value(usage_for(of, &extensions.home)).unwrap_or_default())
        }
    })
}

async fn serve_page(
    extensions: &Extensions,
    id: &str,
    pane: &str,
    nonce: &str,
    rest: &str,
) -> Response<Body> {
    let granted = extensions
        .grant(nonce)
        .is_some_and(|g| g.extension_id == id && g.pane_id == pane);
    if !granted {
        return not_found();
    }
    let Some(pack) = extensions
        .supervisor
        .store()
        .describe(id)
        .filter(InstalledPack::is_extension)
    else {
        return not_found();
    };
    let Some(file) = page::page_file(&pack, pane, rest) else {
        return not_found();
    };
    let Some(media) = page::media_type(&file) else {
        return not_found();
    };
    let Ok(bytes) = tokio::fs::read(&file).await else {
        return not_found();
    };
    // hyper sends no body to a HEAD request.
    let mut res = Response::new(full(bytes));
    let headers = res.headers_mut();
    for (name, value) in page::page_headers(media, &extensions.ancestors) {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(HeaderName::from_static(name), value);
        }
    }
    res
}

/// Serves pane pages and their bridge on `listener` until vornd stops.
pub async fn serve_pages(extensions: Weak<Extensions>, listener: TcpListener) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(err) => {
                warn!(%err, "a pane page connection was not accepted");
                continue;
            }
        };
        let extensions = extensions.clone();
        tokio::spawn(async move {
            let service = service_fn(move |req| pages(extensions.clone(), req, peer));
            let conn = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service);
            if let Err(err) = conn.await {
                debug!(%peer, %err, "a pane page connection ended with an error");
            }
        });
    }
}

async fn pages(
    extensions: Weak<Extensions>,
    req: Request<Incoming>,
    peer: SocketAddr,
) -> Result<Response<Body>, Infallible> {
    let Some(extensions) = extensions.upgrade() else {
        return Ok(not_found());
    };
    let route = bridge::route(req.method().as_str(), req.uri().path())
        .filter(|r| !matches!(r, Route::Bridge { .. }));
    Ok(match route {
        Some(route) => answer(&extensions, route, req, peer).await,
        None => not_found(),
    })
}

/// Binds the page listener on loopback and serves it; answers its origin.
pub async fn start_pages(extensions: &Arc<Extensions>) -> std::io::Result<String> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let origin = format!("http://{}", listener.local_addr()?);
    extensions.set_page_origin(origin.clone());
    tokio::spawn(serve_pages(Arc::downgrade(extensions), listener));
    Ok(origin)
}

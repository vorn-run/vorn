//! The work model's pages over HTTP: an artifact's version
//! (`/artifact/<id>/<version>?t=`), a gate's review page
//! (`/gate-view/<run>/<node>?t=`), and a workflow's webhook
//! (`/wf-hooks/<workflow>/<token>`).
//!
//! The server's addresses for them stay valid: the server relays these
//! paths here. Pages are served under a policy that lets them reach
//! nothing ([`vorn_work::gates::CSP`]); a webhook is taken from this machine
//! only, and every miss is one 404, so the route confirms nothing about
//! ids, tokens or configuration.

use std::net::SocketAddr;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{json, Map, Value};
use vorn_work::inbox::{Received, Request as Hook};

use super::Work;
use crate::endpoint::{full, Body};
use crate::pair::PEER_HEADER;

/// The largest webhook body taken, the server's own limit.
const MAX_BODY: usize = 1 << 20;

/// The work model's path a request names, its first segment decoded as
/// the server's router decodes it; `None` for any other path.
pub fn route_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix('/')?;
    let (first, tail) = rest.split_once('/')?;
    let first = percent_decode(first);
    matches!(first.as_str(), "artifact" | "gate-view" | "wf-hooks")
        .then(|| format!("/{first}/{tail}"))
}

/// Whether this is one of the work model's paths.
pub fn is_route(path: &str) -> bool {
    route_path(path).is_some()
}

fn json_reply(status: StatusCode, body: Value) -> Response<Body> {
    let mut res = Response::new(full(body.to_string()));
    *res.status_mut() = status;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    res
}

fn page(html: impl Into<Bytes>) -> Response<Body> {
    let mut res = Response::new(full(html));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(vorn_work::gates::CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// The query's string values, decoded.
fn query(req: &Request<Incoming>) -> Map<String, Value> {
    let mut out = Map::new();
    for pair in req
        .uri()
        .query()
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
    {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let decode = |s: &str| percent_decode(&s.replace('+', " "));
        out.insert(decode(k), Value::String(decode(v)));
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The path's segments after its prefix, decoded.
fn segments(path: &str, prefix: &str) -> Vec<String> {
    path.trim_start_matches(prefix)
        .split('/')
        .map(percent_decode)
        .collect()
}

/// Answers a request to one of the work model's paths.
pub async fn answer(work: &Work, req: Request<Incoming>, peer: SocketAddr) -> Response<Body> {
    let path = route_path(req.uri().path()).unwrap_or_default();
    let token = query(&req)
        .get("t")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if let Some(rest) = path.strip_prefix("/artifact/") {
        let parts = segments(rest, "");
        let html = match (req.method(), parts.as_slice()) {
            (&Method::GET, [id, version]) if !token.is_empty() => match version.parse::<u32>() {
                Ok(n) if n > 0 => work.artifact_page(id, n, &token).await,
                _ => None,
            },
            _ => None,
        };
        return match html {
            Some(html) => page(html),
            None => json_reply(
                StatusCode::NOT_FOUND,
                json!({ "error": "No artifact here" }),
            ),
        };
    }
    if let Some(rest) = path.strip_prefix("/gate-view/") {
        let parts = segments(rest, "");
        let file = match (req.method(), parts.as_slice()) {
            (&Method::GET, [run, node]) if !token.is_empty() => {
                work.gate_page(run, node, &token).await
            }
            _ => None,
        };
        let html = match file {
            Some(file) => tokio::fs::read(file).await.ok(),
            None => None,
        };
        return match html {
            Some(html) => page(html),
            None => json_reply(
                StatusCode::NOT_FOUND,
                json!({ "error": "No review page here" }),
            ),
        };
    }
    webhook(work, req, peer, &path).await
}

async fn webhook(
    work: &Work,
    req: Request<Incoming>,
    peer: SocketAddr,
    path: &str,
) -> Response<Body> {
    let relayed_from = req
        .headers()
        .get(PEER_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let local = peer.ip().is_loopback()
        && relayed_from.as_deref().is_none_or(|p| {
            p.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
                || p == "::ffff:127.0.0.1"
        });
    if !local {
        return json_reply(
            StatusCode::FORBIDDEN,
            json!({ "error": "Local machine only" }),
        );
    }
    let not_found = || json_reply(StatusCode::NOT_FOUND, json!({ "error": "Not found" }));
    let parts = segments(path.trim_start_matches("/wf-hooks/"), "");
    let [workflow, token] = parts.as_slice() else {
        return not_found();
    };
    let method = req.method().as_str().to_owned();
    if method != "GET" && method != "POST" {
        return not_found();
    }
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .filter(|(name, _)| name.as_str() != PEER_HEADER)
        .filter_map(|(name, value)| {
            Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
        })
        .collect();
    let delivery_id = req
        .headers()
        .get("idempotency-key")
        .or_else(|| req.headers().get("x-vorn-delivery"))
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let json_body = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.contains("json"));
    let query = query(&req);
    let bytes = match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => {
            return json_reply(
                StatusCode::PAYLOAD_TOO_LARGE,
                json!({ "error": "Request body is too large" }),
            )
        }
    };
    let body = if bytes.is_empty() {
        Value::Null
    } else if json_body {
        match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => {
                return json_reply(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": "Body is not valid JSON" }),
                )
            }
        }
    } else {
        Value::String(String::from_utf8_lossy(&bytes).into_owned())
    };
    let hook = Hook {
        method,
        headers,
        query,
        body,
        delivery_id,
    };
    match work.webhook(workflow, token, hook).await {
        Received::Queued | Received::Repeat => {
            json_reply(StatusCode::ACCEPTED, json!({ "accepted": true }))
        }
        Received::NotFound => not_found(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_paths_and_queries() {
        assert_eq!(percent_decode("a%20b%2Fc%zz%"), "a b/c%zz%");
        assert_eq!(percent_decode("é%C3%A9"), "éé");
        assert_eq!(segments("r%2F1/node", ""), ["r/1", "node"]);
        assert!(is_route("/artifact/a/1"));
        assert_eq!(
            route_path("/%61rtifact/a/1").as_deref(),
            Some("/artifact/a/1")
        );
        assert!(is_route("/gate-view/r/n"));
        assert!(!is_route("/artifact"));
        assert!(is_route("/wf-hooks/w/t"));
        assert!(!is_route("/ws"));
    }
}

//! The phone's half of pairing, once the `pairing` group is vornd's:
//! `POST /api/pair/redeem` and `POST /api/pair/poll`, answered as the
//! server's routes answer them ([`crate::native::reach`]).
//!
//! A phone reaches the server, not vornd, which listens on loopback; the
//! server relays these two requests here with the phone's address in
//! `x-vorn-peer`. What the server would refuse before its handler ran (a
//! body that is not plain JSON, or one its parser rejects) goes to the
//! server as it is, marked so it does not relay it back, and so does every
//! request while the server is not listening on the app's channel, when
//! pairing is the server's ([`Native::holds_pairing`]).

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Request, Response, StatusCode};
use serde_json::{json, Value};

use crate::groups::Counted;
use crate::native::Native;
use crate::proxy::{full, Body, Daemon};

pub const REDEEM: &str = "/api/pair/redeem";
pub const POLL: &str = "/api/pair/poll";

/// Where a relayed request came from, which only a peer on this machine
/// may say.
pub const PEER_HEADER: &str = "x-vorn-peer";

/// Set on a pairing request vornd hands the server, so the server answers
/// it itself rather than relaying it back.
pub const FORWARDED_HEADER: &str = "x-vornd-forwarded";

/// The server's own limit for a body.
const MAX_BODY: usize = 1 << 20;

pub fn is_pair_path(path: &str) -> bool {
    path == REDEEM || path == POLL
}

/// Answers a pairing request, or hands it to the server.
pub async fn answer(
    daemon: &Daemon,
    native: &Arc<Native>,
    req: Request<Incoming>,
    peer: SocketAddr,
) -> Response<Body> {
    let (mut parts, body) = req.into_parts();
    let bytes = match Limited::new(body, MAX_BODY).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return too_large(),
    };
    let method = if parts.uri.path() == REDEEM {
        "pairing:redeem"
    } else {
        "pairing:poll"
    };
    let body = plain_json(&parts.headers, &bytes).filter(|_| native.holds_pairing());
    let Some(body) = body else {
        daemon.groups().count(method, Counted::Forwarded);
        parts
            .headers
            .insert(FORWARDED_HEADER, HeaderValue::from_static("1"));
        return daemon.forward_buffered(parts, bytes).await;
    };
    let address = address_of(&parts.headers, peer);
    let redeem = method == "pairing:redeem";
    let native = Arc::clone(native);
    let answered = tokio::task::spawn_blocking(move || {
        if redeem {
            native.pair_redeem(&body, &address)
        } else {
            native.pair_poll(&body)
        }
    })
    .await;
    let (status, body) = answered.unwrap_or_else(|_| {
        (
            500,
            json!({ "statusCode": 500, "error": "Internal Server Error", "message": "pairing failed" }),
        )
    });
    daemon.groups().count(method, Counted::Native);
    json_response(status, &body)
}

/// The body, when the server's parser would read it as this JSON: an
/// `application/json` body that parses and names no prototype key, which
/// the server's parser refuses.
fn plain_json(headers: &HeaderMap, bytes: &Bytes) -> Option<Value> {
    let content_type = headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
    let essence = content_type.split(';').next().unwrap_or("").trim();
    if !essence.eq_ignore_ascii_case("application/json") {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    if text.trim().is_empty() || text.contains("__proto__") || text.contains("constructor") {
        return None;
    }
    serde_json::from_str(text).ok()
}

/// The phone's address: the one the server relayed, or the peer's own.
fn address_of(headers: &HeaderMap, peer: SocketAddr) -> String {
    let said = headers
        .get(PEER_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty());
    match said {
        Some(s) if peer.ip().is_loopback() => s.to_owned(),
        _ => peer.ip().to_string(),
    }
}

fn json_response(status: u16, body: &Value) -> Response<Body> {
    let mut res = Response::new(full(body.to_string()));
    *res.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    res
}

/// What the server answers a body over its limit.
fn too_large() -> Response<Body> {
    json_response(
        413,
        &json!({
            "statusCode": 413,
            "code": "FST_ERR_CTP_BODY_TOO_LARGE",
            "error": "Payload Too Large",
            "message": "Request body is too large",
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_headers(ct: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_str(ct).unwrap());
        h
    }

    #[test]
    fn answers_only_bodies_the_server_would_parse_the_same() {
        let body = Bytes::from_static(br#"{"code":"ABCD-EFGH"}"#);
        assert!(plain_json(&json_headers("application/json"), &body).is_some());
        assert!(plain_json(&json_headers("Application/JSON; charset=utf-8"), &body).is_some());
        assert!(plain_json(&json_headers("text/plain; application/json"), &body).is_none());
        assert!(plain_json(&HeaderMap::new(), &body).is_none());
        let h = json_headers("application/json");
        assert!(plain_json(&h, &Bytes::from_static(b"")).is_none());
        assert!(plain_json(&h, &Bytes::from_static(b"{nope")).is_none());
        assert!(plain_json(&h, &Bytes::from_static(br#"{"__proto__":{}}"#)).is_none());
        assert!(plain_json(&h, &Bytes::from_static(br#"{"constructor":{}}"#)).is_none());
    }

    #[test]
    fn only_a_peer_on_this_machine_may_say_where_the_phone_is() {
        let mut h = HeaderMap::new();
        h.insert(PEER_HEADER, HeaderValue::from_static("192.168.1.7"));
        let local: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let far: SocketAddr = "10.0.0.3:5000".parse().unwrap();
        assert_eq!(address_of(&h, local), "192.168.1.7");
        assert_eq!(address_of(&h, far), "10.0.0.3");
        assert_eq!(address_of(&HeaderMap::new(), local), "127.0.0.1");
    }
}

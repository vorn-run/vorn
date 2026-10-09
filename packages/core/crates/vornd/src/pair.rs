//! The phone's half of pairing: `POST /api/pair/redeem` and `POST /api/pair/poll`.

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Request, Response, StatusCode};
use serde_json::{json, Value};

use crate::endpoint::{full, Body};
use crate::native::Native;

pub const REDEEM: &str = "/api/pair/redeem";
pub const POLL: &str = "/api/pair/poll";

/// Where a relayed request came from, which only a peer on this machine
/// may say.
pub const PEER_HEADER: &str = "x-vorn-peer";

/// The limit for a body.
const MAX_BODY: usize = 1 << 20;

pub fn is_pair_path(path: &str) -> bool {
    path == REDEEM || path == POLL
}

/// Answers a pairing request.
pub async fn answer(
    native: &Arc<Native>,
    req: Request<Incoming>,
    peer: SocketAddr,
) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let bytes = match Limited::new(body, MAX_BODY).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return too_large(),
    };
    let method = if parts.uri.path() == REDEEM {
        "pairing:redeem"
    } else {
        "pairing:poll"
    };
    let Some(body) = plain_json(&parts.headers, &bytes) else {
        return unreadable(&parts.headers);
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
    json_response(status, &body)
}

/// The answer to a body that is not JSON, or JSON that does not parse.
fn unreadable(headers: &HeaderMap) -> Response<Body> {
    let json_typed = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("application/json"));
    if json_typed {
        json_response(
            400,
            &json!({ "statusCode": 400, "error": "Bad Request", "message": "Body is not valid JSON" }),
        )
    } else {
        json_response(415, &json!({ "error": "Expected application/json" }))
    }
}

/// The body, when it is `application/json` that parses and names no prototype key.
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

/// The phone's address: the one a peer on this machine relayed, or the peer's own.
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

/// The answer to a body over the limit.
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
    fn reads_only_plain_json_bodies() {
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

//! `/mcp`, Vorn's MCP server: who may use [`vorn_mcp`]'s tools, and their way back to vornd ([`loopback`]).
//!
//! Who may call: an agent on this machine, holding the local credential.
//! - A request with an `Origin` is refused (403). Agents are not browsers,
//!   and a page in one must not reach the tools by DNS rebinding or a
//!   cross-site POST, which is all an `Origin` can come from.
//! - The credential is the desktop's launch token, the same secret as
//!   `<dataDir>/local-token`, compared in constant time (401 otherwise).
//!   Device tokens do not open it: a paired phone has no business running
//!   an agent's tools. While vornd knows no token, nothing can be checked,
//!   so every request is refused (503).
//!
//! The agent's directory and session come in `Vorn-Cwd` (percent-encoded) and `Vorn-Session-Id`.

mod loopback;

use std::net::SocketAddr;
use std::sync::LazyLock;

use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Request, Response, StatusCode};
use serde_json::{json, Value};
use vorn_mcp::http::{self, Transport};
use vorn_mcp::{Caller, Server};

use crate::endpoint::{full, Body};

pub use loopback::Loopback;

pub const PATH: &str = "/mcp";

/// A tool call carries at most a workflow or a file's worth of JSON.
const MAX_BODY: usize = 16 << 20;

const CWD_HEADER: &str = "vorn-cwd";
const SESSION_HEADER: &str = "vorn-session-id";

/// `packages/mcp`'s version, which the stdio relay reports too.
static VERSION: LazyLock<String> = LazyLock::new(|| {
    serde_json::from_str::<Value>(include_str!("../../../../mcp/package.json"))
        .ok()
        .and_then(|p| p["version"].as_str().map(str::to_owned))
        .unwrap_or_default()
});

/// The MCP server vornd runs: the transport and its sessions, and the
/// socket its tools reach vornd through.
pub struct Mcp {
    transport: Transport,
    rpc: Loopback,
}

impl Mcp {
    /// An MCP server whose tools call vornd at `addr` with `token`.
    pub fn new(addr: SocketAddr, token: &[u8]) -> Mcp {
        Mcp {
            transport: Transport::new(Server::new(VERSION.as_str())),
            rpc: Loopback::new(addr, token),
        }
    }

    /// Open MCP sessions, for the health report.
    pub fn sessions(&self) -> usize {
        self.transport.session_count()
    }
}

/// Why a request may not use the tools, as the response that says so.
pub fn refusal(headers: &HeaderMap, token: Option<&[u8]>) -> Option<Response<Body>> {
    if headers.contains_key(header::ORIGIN) {
        return Some(error(StatusCode::FORBIDDEN, "Origin not allowed"));
    }
    let Some(token) = token else {
        return Some(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "vornd has no local credential to check against",
        ));
    };
    let presented = headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes);
    if !crate::endpoint::is_desktop_credential(presented, token) {
        let mut res = error(StatusCode::UNAUTHORIZED, "Unauthorized");
        res.headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return Some(res);
    }
    None
}

/// Answers one request to `/mcp` that [`refusal`] let through.
pub async fn answer(mcp: &Mcp, req: Request<Incoming>) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let bytes = match Limited::new(body, MAX_BODY).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Request body is too large"),
    };
    let headers = &parts.headers;
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let caller = caller(text(CWD_HEADER), text(SESSION_HEADER));
    let request = http::Request {
        method: parts.method.as_str(),
        accept: text(header::ACCEPT.as_str()),
        content_type: text(header::CONTENT_TYPE.as_str()),
        session_id: text("mcp-session-id"),
        protocol_version: text("mcp-protocol-version"),
        body: &bytes,
    };
    let answered = mcp.transport.handle(&mcp.rpc, &caller, request).await;
    let mut res = Response::new(full(answered.body.unwrap_or_default()));
    *res.status_mut() = StatusCode::from_u16(answered.status).unwrap_or(StatusCode::OK);
    for (name, value) in answered.headers {
        if let Ok(value) = HeaderValue::from_str(&value) {
            res.headers_mut().insert(name, value);
        }
    }
    res
}

/// Who is calling, from the relay's headers. Without a directory the
/// agent's is not known, and vornd's own stands in, as the TypeScript
/// server would use its own.
fn caller(cwd: Option<&str>, session: Option<&str>) -> Caller {
    let cwd = cwd
        .map(percent_decode)
        .filter(|c| !c.is_empty())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|d| d.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "/".to_owned());
    Caller {
        cwd,
        session: session.filter(|s| !s.is_empty()).map(str::to_owned),
    }
}

/// `decodeURIComponent`, lenient: a `%` that starts no escape is kept.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A refusal as a JSON-RPC error with no id, as the transport words its own.
fn error(status: StatusCode, message: &str) -> Response<Body> {
    let body =
        json!({ "jsonrpc": "2.0", "error": { "code": -32000, "message": message }, "id": null });
    let mut res = Response::new(full(body.to_string()));
    *res.status_mut() = status;
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn only_the_local_credential_without_an_origin_gets_in() {
        let token = b"s3cret".as_slice();
        let good = headers(&[("authorization", "Bearer s3cret")]);
        assert!(refusal(&good, Some(token)).is_none());

        let status = |h: &HeaderMap, t: Option<&[u8]>| refusal(h, t).map(|r| r.status());
        let with_origin = headers(&[
            ("authorization", "Bearer s3cret"),
            ("origin", "http://localhost"),
        ]);
        assert_eq!(
            status(&with_origin, Some(token)),
            Some(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            status(&headers(&[]), Some(token)),
            Some(StatusCode::UNAUTHORIZED)
        );
        let wrong = headers(&[("authorization", "Bearer s3creT")]);
        assert_eq!(status(&wrong, Some(token)), Some(StatusCode::UNAUTHORIZED));
        assert_eq!(status(&good, None), Some(StatusCode::SERVICE_UNAVAILABLE));
    }

    #[test]
    fn the_caller_comes_from_the_relays_headers() {
        let caller = caller(Some("/home/me/my%20app"), Some("s-1"));
        assert_eq!(caller.cwd, "/home/me/my app");
        assert_eq!(caller.session.as_deref(), Some("s-1"));
        assert!(super::caller(None, Some("")).session.is_none());
    }

    #[test]
    fn percent_decoding_keeps_what_is_not_an_escape() {
        assert_eq!(percent_decode("/a%2Fb%zz%4"), "/a/b%zz%4");
        assert_eq!(percent_decode("/caf%C3%A9"), "/café");
    }

    #[test]
    fn reports_the_typescript_packages_version() {
        assert!(!VERSION.is_empty());
    }
}

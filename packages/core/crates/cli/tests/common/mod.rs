//! Fakes the command talks to: a Vorn server's WebSocket and vornd's
//! Streamable HTTP `/mcp`, each scripted by the test that starts it.

#![allow(dead_code)]

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as WsRequest, Response as WsResponse,
};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

pub const CREDENTIAL: &str = "test-credential";

/// What the fake server does with a call.
pub enum Answer {
    Result(Value),
    Error(String),
    /// Close the socket with this code instead of answering.
    Close(u16),
    /// Say nothing.
    Silence,
}

/// A Vorn server's WebSocket that answers each call with `answer(method, params)`.
pub async fn ws_server(answer: impl Fn(&str, &Value) -> Answer + Send + Sync + 'static) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let answer = Arc::new(answer);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let answer = answer.clone();
            tokio::spawn(async move {
                // The shape tungstenite's handshake callback takes; its error is theirs.
                #[allow(clippy::result_large_err)]
                let check =
                    |req: &WsRequest, res: WsResponse| -> Result<WsResponse, ErrorResponse> {
                        let auth = req
                            .headers()
                            .get("authorization")
                            .and_then(|v| v.to_str().ok());
                        assert_eq!(auth, Some(format!("Bearer {CREDENTIAL}").as_str()));
                        Ok(res)
                    };
                let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(stream, check).await else {
                    return;
                };
                while let Some(Ok(message)) = ws.next().await {
                    let Message::Text(text) = message else {
                        continue;
                    };
                    let call: Value = serde_json::from_str(&text).unwrap();
                    let Some(id) = call.get("id").cloned() else {
                        continue;
                    };
                    let method = call["method"].as_str().unwrap_or_default().to_owned();
                    let params = call.get("params").cloned().unwrap_or(Value::Null);
                    let reply = match answer(&method, &params) {
                        Answer::Result(result) => {
                            serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})
                        }
                        Answer::Error(message) => serde_json::json!({
                            "jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}
                        }),
                        Answer::Close(code) => {
                            let _ = ws
                                .close(Some(CloseFrame {
                                    code: CloseCode::from(code),
                                    reason: "".into(),
                                }))
                                .await;
                            return;
                        }
                        Answer::Silence => continue,
                    };
                    // A broadcast first, which is not the answer.
                    let _ = ws
                        .send(Message::text(
                            r#"{"jsonrpc":"2.0","method":"terminal:data"}"#,
                        ))
                        .await;
                    let _ = ws.send(Message::text(reply.to_string())).await;
                }
            });
        }
    });
    port
}

/// A data directory announcing a server on `port`.
pub fn announce(dir: &Path, port: u16) {
    std::fs::write(dir.join("ws-port"), format!("{{\"port\":{port}}}")).unwrap();
    std::fs::write(dir.join("local-token"), format!("{CREDENTIAL}\n")).unwrap();
}

/// One request `/mcp` received.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub session: Option<String>,
    pub protocol: Option<String>,
    pub accept: Option<String>,
    pub body: String,
}

pub type Log = Arc<Mutex<Vec<Seen>>>;

/// What the fake `/mcp` answers a request with: status, content type,
/// session id to issue, body.
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub session: Option<&'static str>,
    pub body: String,
}

impl Reply {
    pub fn json(body: impl Into<String>) -> Reply {
        Reply {
            status: 200,
            content_type: "application/json",
            session: None,
            body: body.into(),
        }
    }

    pub fn events(body: impl Into<String>) -> Reply {
        Reply {
            status: 200,
            content_type: "text/event-stream",
            session: None,
            body: body.into(),
        }
    }

    pub fn status(status: u16, body: impl Into<String>) -> Reply {
        Reply {
            status,
            content_type: "text/plain",
            session: None,
            body: body.into(),
        }
    }
}

fn header(req: &Request<Incoming>, name: &str) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// vornd's `/mcp`, answering each request with `reply(http method, JSON-RPC body)`.
pub async fn mcp_server(
    reply: impl Fn(&str, &Value) -> Reply + Send + Sync + 'static,
) -> (u16, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log: Log = Arc::default();
    let reply = Arc::new(reply);
    let seen = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)): std::io::Result<(_, SocketAddr)> = listener.accept().await else {
                return;
            };
            let reply = reply.clone();
            let seen = seen.clone();
            let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                let reply = reply.clone();
                let seen = seen.clone();
                async move {
                    let auth = header(&req, "authorization");
                    let entry = Seen {
                        method: req.method().to_string(),
                        session: header(&req, "mcp-session-id"),
                        protocol: header(&req, "mcp-protocol-version"),
                        accept: header(&req, "accept"),
                        body: String::new(),
                    };
                    let path_ok = req.uri().path() == "/mcp";
                    let body = req.into_body().collect().await.unwrap().to_bytes();
                    let body = String::from_utf8_lossy(&body).into_owned();
                    seen.lock().unwrap().push(Seen {
                        body: body.clone(),
                        ..entry.clone()
                    });
                    let answer = if auth.as_deref() != Some(&format!("Bearer {CREDENTIAL}")) {
                        Reply::status(401, "who are you")
                    } else if !path_ok {
                        Reply::status(404, "")
                    } else {
                        let parsed = serde_json::from_str(&body).unwrap_or(Value::Null);
                        reply(&entry.method, &parsed)
                    };
                    let mut response = Response::builder()
                        .status(answer.status)
                        .header("content-type", answer.content_type);
                    if let Some(session) = answer.session {
                        response = response.header("mcp-session-id", session);
                    }
                    Ok::<_, Infallible>(response.body(Full::new(Bytes::from(answer.body))).unwrap())
                }
            });
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (port, log)
}

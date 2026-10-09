//! A client of vornd's WebSocket, which starts sessions with `vornd:spawn` (`--debug-spawn`).

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::error::Error;

/// An answer to a call: its id, and the result or the error's message.
pub type Answer = (u64, Result<Value, String>);

type Sink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Debug)]
pub struct AppClient {
    wr: Sink,
    answers: mpsc::UnboundedReceiver<Answer>,
}

impl AppClient {
    /// Connects to vornd on `port` with `token`, its local credential.
    pub async fn connect(port: u16, token: &str) -> Result<AppClient, Error> {
        let mut req = format!("ws://127.0.0.1:{port}/ws")
            .into_client_request()
            .map_err(|e| Error::Failed(e.to_string()))?;
        let bearer = format!("Bearer {token}")
            .parse()
            .map_err(|_| Error::Failed("a credential that is not a header value".into()))?;
        req.headers_mut().insert("authorization", bearer);
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| Error::Failed(format!("vornd's WebSocket: {e}")))?;
        let (wr, mut rd) = ws.split();
        let (tx, answers) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = rd.next().await {
                let Message::Text(text) = msg else { continue };
                if let Some(a) = answer(text.as_bytes()) {
                    if tx.send(a).is_err() {
                        return;
                    }
                }
            }
        });
        Ok(AppClient { wr, answers })
    }

    pub async fn call(&mut self, id: u64, method: &str, params: Value) -> Result<(), Error> {
        let text =
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        self.wr
            .send(Message::text(text))
            .await
            .map_err(|_| Error::Closed("vornd's WebSocket"))
    }

    pub async fn answer(&mut self) -> Result<Answer, Error> {
        self.answers
            .recv()
            .await
            .ok_or(Error::Closed("vornd's WebSocket"))
    }
}

/// A JSON-RPC answer, or nothing for notifications and anything else.
fn answer(payload: &[u8]) -> Option<Answer> {
    let v: Value = serde_json::from_slice(payload).ok()?;
    let id = v.get("id")?.as_u64()?;
    if let Some(e) = v.get("error") {
        let message = e.get("message").and_then(Value::as_str).unwrap_or("error");
        return Some((id, Err(message.to_owned())));
    }
    Some((id, Ok(v.get("result")?.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_carry_their_id_and_notifications_are_skipped() {
        let ok = answer(br#"{"jsonrpc":"2.0","id":7,"result":{"id":"s1"}}"#).unwrap();
        assert_eq!(ok, (7, Ok(json!({ "id": "s1" }))));
        let err = answer(br#"{"id":8,"error":{"code":-32000,"message":"no ptys"}}"#).unwrap();
        assert_eq!(err, (8, Err("no ptys".to_owned())));
        assert_eq!(answer(br#"{"method":"server:hello","params":{}}"#), None);
        assert_eq!(answer(b"not json"), None);
    }
}

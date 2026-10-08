//! A client of vornd's app channel, where the app's server starts
//! sessions: 4-byte little-endian length, a kind byte (1 for JSON), then
//! the JSON-RPC text.

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::error::Error;

const KIND_TEXT: u8 = 1;

/// An answer to a call: its id, and the result or the error's message.
pub type Answer = (u64, Result<Value, String>);

#[derive(Debug)]
pub struct AppClient {
    wr: OwnedWriteHalf,
    answers: mpsc::UnboundedReceiver<Answer>,
}

impl AppClient {
    pub async fn connect(endpoint: &str) -> Result<AppClient, Error> {
        let (mut rd, wr) = UnixStream::connect(endpoint).await?.into_split();
        let (tx, answers) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut pending = Vec::new();
            let mut buf = vec![0u8; 64 << 10];
            loop {
                match rd.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => pending.extend_from_slice(&buf[..n]),
                }
                let mut at = 0;
                while let Some((kind, payload, next)) = frame(&pending[at..]) {
                    at += next;
                    if kind != KIND_TEXT {
                        continue;
                    }
                    if let Some(a) = answer(payload) {
                        if tx.send(a).is_err() {
                            return;
                        }
                    }
                }
                pending.drain(..at);
            }
        });
        Ok(AppClient { wr, answers })
    }

    pub async fn call(&mut self, id: u64, method: &str, params: Value) -> Result<(), Error> {
        let text =
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        let mut out = Vec::with_capacity(5 + text.len());
        out.extend_from_slice(&(1 + text.len() as u32).to_le_bytes());
        out.push(KIND_TEXT);
        out.extend_from_slice(text.as_bytes());
        self.wr.write_all(&out).await?;
        Ok(())
    }

    pub async fn answer(&mut self) -> Result<Answer, Error> {
        self.answers
            .recv()
            .await
            .ok_or(Error::Closed("vornd's app channel"))
    }
}

/// The first whole frame in `buf`: its kind, its payload, and where the
/// next one starts.
fn frame(buf: &[u8]) -> Option<(u8, &[u8], usize)> {
    let len = u32::from_le_bytes(buf.get(..4)?.try_into().ok()?) as usize;
    let body = buf.get(4..4 + len)?;
    let (&kind, payload) = body.split_first()?;
    Some((kind, payload, 4 + len))
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

    fn framed(kind: u8, text: &str) -> Vec<u8> {
        let mut out = (1 + text.len() as u32).to_le_bytes().to_vec();
        out.push(kind);
        out.extend_from_slice(text.as_bytes());
        out
    }

    #[test]
    fn frames_are_split_where_their_length_says() {
        let mut buf = framed(1, r#"{"id":1}"#);
        buf.extend(framed(2, "xy"));
        let (kind, payload, next) = frame(&buf).unwrap();
        assert_eq!((kind, payload, next), (1, br#"{"id":1}"#.as_slice(), 13));
        let (kind, payload, _) = frame(&buf[next..]).unwrap();
        assert_eq!((kind, payload), (2, b"xy".as_slice()));
        assert_eq!(frame(&buf[..12]), None);
        assert_eq!(frame(&[]), None);
    }

    #[test]
    fn answers_carry_their_id_and_notifications_are_skipped() {
        let ok = answer(br#"{"jsonrpc":"2.0","id":7,"result":{"id":"s1"}}"#).unwrap();
        assert_eq!(ok, (7, Ok(json!({ "id": "s1" }))));
        let err = answer(br#"{"id":8,"error":{"code":-32000,"message":"no ptys"}}"#).unwrap();
        assert_eq!(err, (8, Err("no ptys".to_owned())));
        assert_eq!(answer(br#"{"method":"vornd:activity","params":{}}"#), None);
        assert_eq!(answer(b"not json"), None);
    }
}

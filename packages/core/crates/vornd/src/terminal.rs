//! The terminal calls vornd answers itself, for the sessions it holds.
//!
//! Routing is per session, not per group: `terminal:attach`, `write`,
//! `resize`, `readScrollback` and `readOutput` naming a session the engine
//! holds never reach the Node server; the same calls for any other session
//! (Node's own PTYs) are forwarded as they always were. What answers them is
//! in [`crate::streams`]; this module only reads the calls.
//!
//! `vornd:spawn` starts a session in sessiond through the engine. It exists
//! for tests until the app creates sessions through vornd, and is answered
//! only when vornd was started with `--debug-spawn`.

use serde_json::{json, Value};
use vorn_sessiond_wire::{Io, SpawnSpec, Stdin};
use vorn_term_proto::Cursor;

use crate::engine::Engine;
use crate::streams::{answer, refuse, Forwarder};

/// The calls vornd may answer, cheap to test for before parsing a frame.
const NATIVE: [&str; 6] = [
    "\"terminal:attach\"",
    "\"terminal:write\"",
    "\"terminal:resize\"",
    "\"terminal:readScrollback\"",
    "\"terminal:readOutput\"",
    "\"vornd:spawn\"",
];

/// Answers `text` from client connection `conn` if it is a terminal call
/// for a session vornd holds. False when it is for the server.
pub fn handle(
    engine: &std::sync::Arc<Engine>,
    conn: u64,
    reply: &Forwarder,
    text: &str,
    allow_spawn: bool,
) -> bool {
    if !NATIVE.iter().any(|m| text.contains(m)) {
        return false;
    }
    let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    let Some(method) = frame.get("method").and_then(Value::as_str) else {
        return false;
    };
    let rpc = frame.get("id").cloned();
    let params = frame.get("params").cloned().unwrap_or(Value::Null);
    if method == "vornd:spawn" {
        if !allow_spawn {
            return false;
        }
        spawn(engine, reply, rpc, &params);
        return true;
    }
    let Some(session) = params.get("id").and_then(Value::as_str) else {
        return false;
    };
    let streams = engine.streams();
    if !streams.holds(session) {
        return false;
    }
    match method {
        "terminal:attach" => {
            let Some(rpc) = rpc else { return true };
            let cursor = params.get("cursor").and_then(cursor_of);
            engine.perform(streams.attach(conn, session, rpc, cursor));
        }
        "terminal:readScrollback" => {
            let Some(rpc) = rpc else { return true };
            engine.perform(streams.read_scrollback(conn, session, rpc));
        }
        "terminal:readOutput" => {
            let Some(rpc) = rpc else { return true };
            let lines = params
                .get("lines")
                .and_then(Value::as_u64)
                .map_or(u32::MAX, |n| u32::try_from(n).unwrap_or(u32::MAX));
            engine.perform(streams.read_output(conn, session, rpc, lines));
        }
        "terminal:write" => {
            let done = match params.get("data").and_then(Value::as_str) {
                Some(data) => engine.write(session, data.as_bytes().to_vec()),
                None => Err("terminal:write needs data".to_owned()),
            };
            settle(reply, rpc, done);
        }
        "terminal:resize" => {
            let size = |k: &str| {
                params
                    .get(k)
                    .and_then(Value::as_u64)
                    .and_then(|n| u16::try_from(n).ok())
                    .filter(|&n| n > 0)
            };
            let done = match (size("cols"), size("rows")) {
                (Some(cols), Some(rows)) => engine.resize(session, cols, rows),
                _ => Err("terminal:resize needs cols and rows from 1 to 65535".to_owned()),
            };
            settle(reply, rpc, done);
        }
        _ => return false,
    }
    true
}

/// Answers a call that was sent as a request; a notification gets nothing.
fn settle(reply: &Forwarder, rpc: Option<Value>, done: Result<(), String>) {
    let Some(rpc) = rpc else { return };
    match done {
        Ok(()) => reply.send_now(&answer(&rpc, Value::Null)),
        Err(e) => reply.send_now(&refuse(&rpc, &e)),
    }
}

/// A cursor as clients send it: `{epoch, nextRseq, nextOffset}`.
fn cursor_of(v: &Value) -> Option<Cursor> {
    Some(Cursor {
        epoch: u32::try_from(v.get("epoch")?.as_u64()?).ok()?,
        next_rseq: v.get("nextRseq")?.as_u64()?,
        next_offset: v.get("nextOffset")?.as_u64()?,
    })
}

/// `vornd:spawn {argv, cwd?, env?, cols?, rows?, piped?}`: answers `{id}`.
fn spawn(engine: &std::sync::Arc<Engine>, reply: &Forwarder, rpc: Option<Value>, p: &Value) {
    let argv: Vec<String> = p
        .get("argv")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let size = |k: &str, d: u16| {
        p.get(k)
            .and_then(Value::as_u64)
            .and_then(|n| u16::try_from(n).ok())
            .unwrap_or(d)
    };
    let io = if p.get("piped").and_then(Value::as_bool) == Some(true) {
        Io::Piped { stdin: Stdin::Pipe }
    } else {
        Io::Pty {
            cols: size("cols", 80),
            rows: size("rows", 24),
        }
    };
    let env = p
        .get("env")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let spec = SpawnSpec {
        argv,
        cwd: p.get("cwd").and_then(Value::as_str).map_or_else(
            || std::env::temp_dir().to_string_lossy().into_owned(),
            str::to_owned,
        ),
        env,
        io,
        ring_bytes: p
            .get("ringBytes")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
    };
    let (engine, reply) = (std::sync::Arc::clone(engine), reply.clone());
    tokio::spawn(async move {
        let done = if spec.argv.is_empty() {
            Err("vornd:spawn needs argv".to_owned())
        } else {
            engine.spawn(spec).await
        };
        if let Some(rpc) = rpc {
            match done {
                Ok(id) => reply.send_now(&answer(&rpc, json!({ "id": id }))),
                Err(e) => reply.send_now(&refuse(&rpc, &e)),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_cursor_and_refuses_half_of_one() {
        let c = cursor_of(&json!({ "epoch": 2, "nextRseq": 9, "nextOffset": 120 }));
        assert_eq!(
            c,
            Some(Cursor {
                epoch: 2,
                next_rseq: 9,
                next_offset: 120
            })
        );
        // An offset alone is never a resume token.
        assert_eq!(cursor_of(&json!({ "nextOffset": 120 })), None);
        assert_eq!(
            cursor_of(&json!({ "epoch": -1, "nextRseq": 0, "nextOffset": 0 })),
            None
        );
    }
}

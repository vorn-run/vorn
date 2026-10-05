//! The terminal calls vornd answers itself, for the sessions it holds.
//!
//! Routing is per session, not per group: `terminal:attach`, `write`,
//! `resize`, `readScrollback` and `readOutput` naming a session the engine
//! holds never reach the Node server; the same calls for any other session
//! (Node's own PTYs) are forwarded as they always were. What answers them is
//! in [`crate::streams`]; this module only reads the calls.
//!
//! The size of a held session is the size rule's ([`crate::size`]). A
//! client reports `terminal:viewport {id, cols, rows}` and
//! `terminal:presence {id, state}` (`active`, `watching` or `away`), and may
//! send `terminal:takeSize {id}` ("Fit to this device") and
//! `terminal:lockSize {id, locked}`. `terminal:resize {id, cols, rows}`, which
//! an older client sends on every fit, is taken as a TakeSize with that
//! size, so such a client still works the way it did.
//!
//! Each of these calls, `terminal:attach` and `terminal:write` included, may
//! name the pane it comes from with `pane` (a number): the desktop's windows
//! share one connection, and the size rule counts each pane as its own client.
//! A lock another client holds is refused, in the answer to the request.
//!
//! `vornd:spawn` starts a session in sessiond through the engine. The app's
//! server sends it on its own channel ([`crate::control`]), naming each
//! session with its own id ([`crate::names`]); a client may send it only
//! when vornd was started with `--debug-spawn`, which tests use. The app's
//! own calls never count toward the size rule: the server writing to a
//! session for a workflow is not a person typing into it.

use std::time::Instant;

use serde_json::{json, Value};
use vorn_sessiond_wire::{Io, SpawnSpec, Stdin};
use vorn_size::{Presence, Size};
use vorn_term_proto::Cursor;

use crate::engine::Engine;
use crate::size::{Ev, Who};
use crate::streams::{answer, refuse, Forwarder};

/// The calls vornd may answer, cheap to test for before parsing a frame.
const NATIVE: [&str; 10] = [
    "\"terminal:attach\"",
    "\"terminal:write\"",
    "\"terminal:resize\"",
    "\"terminal:readScrollback\"",
    "\"terminal:readOutput\"",
    "\"terminal:viewport\"",
    "\"terminal:presence\"",
    "\"terminal:takeSize\"",
    "\"terminal:lockSize\"",
    "\"vornd:spawn\"",
];

/// Who a call comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Caller {
    /// A client: the renderer, the web client, the phone.
    Client { allow_spawn: bool },
    /// The app's server, on its own channel.
    App,
}

/// Answers `text` from client connection `conn` if it is a terminal call
/// for a session vornd holds. False when it is for the server.
pub fn handle(
    engine: &std::sync::Arc<Engine>,
    conn: u64,
    reply: &Forwarder,
    text: &str,
    allow_spawn: bool,
) -> bool {
    route(engine, conn, reply, text, Caller::Client { allow_spawn })
}

/// Answers `text` from the app's server on connection `conn` if it is a
/// terminal call for a session vornd holds, or `vornd:spawn`.
pub fn handle_for_app(
    engine: &std::sync::Arc<Engine>,
    conn: u64,
    reply: &Forwarder,
    text: &str,
) -> bool {
    route(engine, conn, reply, text, Caller::App)
}

fn route(
    engine: &std::sync::Arc<Engine>,
    conn: u64,
    reply: &Forwarder,
    text: &str,
    caller: Caller,
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
        if caller == (Caller::Client { allow_spawn: false }) {
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
    let sizes = engine.sizes();
    // The app's calls leave the size rule alone.
    let counted = matches!(caller, Caller::Client { .. });
    // 0 when the client names no pane: the connection is then one client.
    let pane = params.get("pane").and_then(Value::as_u64).unwrap_or(0);
    let who = Who::Bytes { conn, pane };
    let now = Instant::now();
    match method {
        "terminal:attach" => {
            let Some(rpc) = rpc else { return true };
            let cursor = params.get("cursor").and_then(cursor_of);
            if counted {
                let ev = Ev::Attach {
                    viewport: size_of(&params),
                    presence: Presence::Watching,
                };
                sizes.on(session, who, ev, now);
            }
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
                Some(data) => {
                    // A person's typing takes the size; focus and scroll
                    // reports do not.
                    if counted && vorn_size::typed(data.as_bytes()) {
                        sizes.on(session, who, Ev::Input, now);
                    }
                    engine.write(session, data.as_bytes().to_vec())
                }
                None => Err("terminal:write needs data".to_owned()),
            };
            settle(reply, rpc, done);
        }
        "terminal:resize" => {
            let done = match size_of(&params) {
                // The app's resize is applied as it is, past the rule.
                Some(size) if !counted => engine.resize(session, size.cols, size.rows),
                Some(size) => {
                    sizes.on(session, who, Ev::TakeSize(Some(size)), now);
                    Ok(())
                }
                None => Err("terminal:resize needs cols and rows from 1 to 65535".to_owned()),
            };
            settle(reply, rpc, done);
        }
        "terminal:viewport" => {
            let done = match size_of(&params) {
                Some(size) => {
                    sizes.on(session, who, Ev::Viewport(size), now);
                    Ok(())
                }
                None => Err("terminal:viewport needs cols and rows from 1 to 65535".to_owned()),
            };
            settle(reply, rpc, done);
        }
        "terminal:presence" => {
            let state = match params.get("state").and_then(Value::as_str) {
                Some("active") => Ok(Presence::Active),
                Some("watching") => Ok(Presence::Watching),
                Some("away") => Ok(Presence::Away),
                _ => Err("terminal:presence needs a state: active, watching or away".to_owned()),
            };
            let done = state.map(|state| sizes.on(session, who, Ev::Presence(state), now));
            settle(reply, rpc, done);
        }
        "terminal:takeSize" => {
            sizes.on(session, who, Ev::TakeSize(size_of(&params)), now);
            settle(reply, rpc, Ok(()));
        }
        "terminal:lockSize" => {
            let done = match params.get("locked").and_then(Value::as_bool) {
                Some(locked) => sizes.lock(session, who, locked, now),
                None => Err("terminal:lockSize needs locked: true or false".to_owned()),
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

/// `cols` and `rows` from a call's params, each from 1 to 65535.
fn size_of(params: &Value) -> Option<Size> {
    let n = |k: &str| {
        params
            .get(k)
            .and_then(Value::as_u64)
            .and_then(|n| u16::try_from(n).ok())
            .filter(|&n| n > 0)
    };
    Some(Size::new(n("cols")?, n("rows")?))
}

/// A cursor as clients send it: `{epoch, nextRseq, nextOffset}`.
fn cursor_of(v: &Value) -> Option<Cursor> {
    Some(Cursor {
        epoch: u32::try_from(v.get("epoch")?.as_u64()?).ok()?,
        next_rseq: v.get("nextRseq")?.as_u64()?,
        next_offset: v.get("nextOffset")?.as_u64()?,
    })
}

/// `vornd:spawn {argv, cwd?, env?, cols?, rows?, piped?, name?}`: answers
/// `{id, pid, epoch}`, `id` being the name when one was given.
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
    let name = p.get("name").and_then(Value::as_str).map(str::to_owned);
    let (engine, reply) = (std::sync::Arc::clone(engine), reply.clone());
    tokio::spawn(async move {
        let done = if spec.argv.is_empty() {
            Err("vornd:spawn needs argv".to_owned())
        } else {
            engine.spawn_as(spec, name).await
        };
        if let Some(rpc) = rpc {
            match done {
                Ok(s) => reply.send_now(&answer(
                    &rpc,
                    json!({ "id": s.id, "pid": s.pid, "epoch": s.epoch }),
                )),
                Err(e) => reply.send_now(&refuse(&rpc, &e)),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_size_only_when_both_sides_fit() {
        assert_eq!(
            size_of(&json!({ "cols": 120, "rows": 40 })),
            Some(Size::new(120, 40))
        );
        for bad in [
            json!({ "cols": 0, "rows": 40 }),
            json!({ "cols": 70000, "rows": 40 }),
            json!({ "cols": 120 }),
            json!({ "cols": -1, "rows": 2 }),
            json!(null),
        ] {
            assert_eq!(size_of(&bad), None, "{bad}");
        }
    }

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

//! The program tier: an [`Extension`] served over stdio, one JSON-RPC
//! message a line, as Vorn speaks to a program (`wit/stdio.json`).
//!
//! It is the same source the component is built from, so the two tiers
//! differ only in how Vorn runs them. Every call to Vorn blocks on its
//! answer; what Vorn sends in the meantime waits its turn, as the component
//! sees it too.

use std::collections::VecDeque;
use std::io::{BufRead, BufWriter, Write};

use vorn_extension_sdk::serde_json::{self, json, Value};
use vorn_extension_sdk::{dispatch, Ctx, Error, Event, Extension, Host, Init, Level, Op, Request, Response};

/// The JSON-RPC error an `ext-error` travels as.
const EXT_ERROR: i64 = -32000;
const METHOD_NOT_FOUND: i64 = -32601;

/// Serves `E` on this process's stdin and stdout until stdin closes.
pub fn serve<E: Extension>() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve_on::<E>(stdin.lock(), BufWriter::new(stdout.lock()));
}

/// Serves `E` on `input` and `out`; a test drives it on buffers.
pub fn serve_on<E: Extension>(input: impl BufRead, out: impl Write) {
    let mut io = Stdio { input, out, waiting: VecDeque::new(), asked: 0 };
    let mut ext: Option<E> = None;
    while let Some(message) = io.next() {
        let id = message.get("id").cloned();
        let params = &message["params"];
        let answer = match message["method"].as_str().unwrap_or_default() {
            "guest.init" => {
                let c = &params["ctx"];
                let text = |k: &str| c[k].as_str().unwrap_or_default().to_owned();
                let init = Init { id: text("id"), version: text("version"), vorn: text("vorn"), api: text("api") };
                E::init(&init, &mut io).map(|e| {
                    ext = Some(e);
                    Value::Null
                })
            }
            "guest.handle" => match ext.as_mut() {
                Some(ext) => {
                    let ctx = Ctx {
                        caller: params["ctx"]["caller"].as_str().unwrap_or_default().to_owned(),
                        project: params["ctx"]["project"].as_str().map(str::to_owned),
                    };
                    let method = params["method"].as_str().unwrap_or_default();
                    dispatch(ext, &mut io, method, params["args"].clone(), &ctx)
                }
                None => Err(Error::new("start", "guest.handle before guest.init")),
            },
            "guest.on-event" => {
                if let Some(ext) = ext.as_mut() {
                    let batch = params["batch"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|e| Event { name: e["name"].as_str().unwrap_or_default().to_owned(), data: e["data"].clone() })
                        .collect();
                    ext.on_events(&mut io, batch);
                }
                continue;
            }
            other => {
                if let Some(id) = id {
                    io.send(&json!({ "id": id, "error": { "code": METHOD_NOT_FOUND, "message": format!("no {other}") } }));
                }
                continue;
            }
        };
        let Some(id) = id else { continue };
        io.send(&match answer {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(e) => json!({ "id": id, "error": { "code": EXT_ERROR, "message": e.message, "data": { "code": e.code } } }),
        });
    }
}

struct Stdio<R, W> {
    input: R,
    out: W,
    /// What Vorn sent while a call to it waited for its answer.
    waiting: VecDeque<Value>,
    asked: u64,
}

impl<R: BufRead, W: Write> Stdio<R, W> {
    fn send(&mut self, message: &Value) {
        let mut line = message.clone();
        line["jsonrpc"] = json!("2.0");
        // A closed stdout means Vorn is gone; the next read of stdin ends the loop.
        let _ = writeln!(self.out, "{line}").and_then(|()| self.out.flush());
    }

    fn read(&mut self) -> Option<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.input.read_line(&mut line).ok()? == 0 {
                return None;
            }
            if let Ok(message) = serde_json::from_str(line.trim()) {
                return Some(message);
            }
        }
    }

    fn next(&mut self) -> Option<Value> {
        self.waiting.pop_front().or_else(|| self.read())
    }

    /// Asks Vorn `method` and waits for its answer.
    fn ask(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        self.asked += 1;
        let id = format!("h{}", self.asked);
        self.send(&json!({ "id": id, "method": method, "params": params }));
        loop {
            let message = self.read().ok_or_else(|| Error::new("closed", "Vorn closed the pipe"))?;
            if message.get("method").is_some() {
                self.waiting.push_back(message);
                continue;
            }
            if message["id"] != id.as_str() {
                continue;
            }
            if let Some(e) = message.get("error") {
                let code = e["data"]["code"].as_str().unwrap_or("failed");
                return Err(Error::new(code, e["message"].as_str().unwrap_or_default()));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({ "method": method, "params": params }));
    }
}

fn op(op: Op) -> Value {
    match op {
        Op::Upsert(row) => json!({ "upsert": row }),
        Op::Delete(id) => json!({ "delete": id }),
        Op::Clear => json!("clear"),
    }
}

impl<R: BufRead, W: Write> Host for Stdio<R, W> {
    fn call(&mut self, method: &str, args: Value) -> Result<Value, Error> {
        self.ask("host.call", json!({ "method": method, "args": args }))
    }

    fn publish(&mut self, collection: &str, ops: Vec<Op>) {
        let ops: Vec<Value> = ops.into_iter().map(op).collect();
        self.notify("host.publish", json!({ "collection": collection, "ops": ops }));
    }

    fn log(&mut self, level: Level, message: &str) {
        let level = match level {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        };
        self.notify("host.log", json!({ "level": level, "message": message }));
    }

    fn rows(&mut self, collection: &str) -> Result<Vec<Value>, Error> {
        match self.ask("host.rows", json!({ "collection": collection }))? {
            Value::Array(rows) => Ok(rows),
            _ => Ok(Vec::new()),
        }
    }

    fn fetch(&mut self, req: &Request, connection: Option<&str>) -> Result<Response, Error> {
        let answer = self.ask("net.fetch", json!({ "req": req, "connection": connection }))?;
        serde_json::from_value(answer).map_err(|e| Error::new("failed", format!("net.fetch answered {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl Extension for Echo {
        fn init(_: &Init, _: &mut dyn Host) -> Result<Echo, Error> {
            Ok(Echo)
        }

        fn handle(&mut self, host: &mut dyn Host, method: &str, args: Value, _: &Ctx) -> Result<Value, Error> {
            match method {
                "ask" => host.call("x.y", args),
                _ => Err(Error::unknown(method)),
            }
        }
    }

    #[test]
    fn answers_in_order_and_holds_what_arrives_while_it_asks() {
        let input = [
            r#"{"jsonrpc":"2.0","id":1,"method":"guest.init","params":{"ctx":{"id":"a","version":"1","vorn":"v","api":"x"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"guest.handle","params":{"method":"ask","args":7,"ctx":{"caller":"client"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"guest.handle","params":{"method":"nope","args":null,"ctx":{"caller":"client"}}}"#,
            r#"{"jsonrpc":"2.0","id":"h1","result":8}"#,
        ]
        .join("\n");
        let mut out = Vec::new();
        serve_on::<Echo>(input.as_bytes(), &mut out);
        let lines: Vec<Value> = String::from_utf8(out)
            .expect("utf-8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("json"))
            .collect();
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[1]["method"], "host.call");
        assert_eq!(lines[2], json!({ "jsonrpc": "2.0", "id": 2, "result": 8 }));
        assert_eq!(lines[3]["error"]["data"]["code"], "unknown-action");
    }
}

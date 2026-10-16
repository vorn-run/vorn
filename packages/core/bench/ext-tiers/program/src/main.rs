//! An extension served as a program: JSON-RPC 2.0, one message a line, on
//! stdin and stdout (`wit/stdio.json`). One thread: a request it makes waits
//! for its answer, and what else arrives meanwhile waits its turn.

use std::collections::VecDeque;
use std::io::{BufRead, BufWriter, Write};

use ext_tiers_logic::{probe, Error, Extension, Host, Op};
use serde_json::{json, Value};

struct Stdio {
    lines: std::io::Lines<std::io::StdinLock<'static>>,
    out: BufWriter<std::io::Stdout>,
    next_id: u64,
    /// Messages that came while it waited for an answer.
    later: VecDeque<Value>,
}

impl Stdio {
    fn send(&mut self, message: &Value) {
        let mut line = message.to_string();
        line.push('\n');
        // A closed stdout means vornd is gone; the next read ends the loop.
        let _ = self.out.write_all(line.as_bytes());
        let _ = self.out.flush();
    }

    fn read(&mut self) -> Option<Value> {
        loop {
            let line = self.lines.next()?.ok()?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str(&line) {
                return Some(v);
            }
        }
    }

    fn next(&mut self) -> Option<Value> {
        self.later.pop_front().or_else(|| self.read())
    }
}

impl Host for Stdio {
    fn call(&mut self, method: &str, args: Value) -> Result<Value, Error> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "host.call",
            "params": { "method": method, "args": args },
        }));
        loop {
            let Some(message) = self.read() else {
                return Err(Error::new("exited", "vornd closed stdin"));
            };
            if message.get("method").is_some() || message["id"] != json!(id) {
                self.later.push_back(message);
                continue;
            }
            if let Some(err) = message.get("error") {
                let code = err["data"]["code"].as_str().unwrap_or("failed");
                let text = err["message"].as_str().unwrap_or_default();
                return Err(Error::new(code, text));
            }
            return Ok(message["result"].clone());
        }
    }

    fn publish(&mut self, collection: &str, ops: Vec<Op>) {
        let ops: Vec<Value> = ops
            .into_iter()
            .map(|op| match op {
                Op::Upsert(row) => json!({ "upsert": row }),
                Op::Delete(id) => json!({ "delete": id }),
                Op::Clear => json!("clear"),
            })
            .collect();
        self.send(&json!({
            "jsonrpc": "2.0", "method": "host.publish",
            "params": { "collection": collection, "ops": ops },
        }));
    }
}

fn answer(id: Value, answer: Result<Value, Error>) -> Value {
    match answer {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32000, "message": e.message, "data": { "code": e.code } },
        }),
    }
}

/// Serves the extension `guest.init` names until stdin closes.
fn main() {
    let mut stdio = Stdio {
        lines: std::io::stdin().lock().lines(),
        out: BufWriter::new(std::io::stdout()),
        next_id: 0,
        later: VecDeque::new(),
    };
    let mut ext: Option<Box<dyn Extension>> = None;
    while let Some(message) = stdio.next() {
        let id = message.get("id").cloned();
        let params = &message["params"];
        match message["method"].as_str() {
            Some("guest.init") => {
                let named = params["ctx"]["id"].as_str().unwrap_or_default();
                let reply = match ext_tiers_logic::by_id(named) {
                    Some(mut e) => {
                        e.init(&mut stdio);
                        ext = Some(e);
                        Ok(Value::Null)
                    }
                    None => Err(Error::new("unknown", format!("no extension {named}"))),
                };
                if let Some(id) = id {
                    stdio.send(&answer(id, reply));
                }
            }
            Some("guest.handle") => {
                let action = params["method"].as_str().unwrap_or_default();
                let args = &params["args"];
                let reply = match (probe(action, args), ext.as_mut()) {
                    (Some(probed), _) => probed,
                    (None, Some(e)) => e.handle(&mut stdio, action, args),
                    (None, None) => Err(Error::new("start", "guest.init came first")),
                };
                if let Some(id) = id {
                    stdio.send(&answer(id, reply));
                }
            }
            Some("guest.on-event") => {
                let batch: Vec<(String, Value)> = params["batch"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        let name = e["name"].as_str().unwrap_or_default().to_owned();
                        (name, e["data"].clone())
                    })
                    .collect();
                if let Some(e) = ext.as_mut() {
                    e.on_events(&mut stdio, &batch);
                }
            }
            _ => {}
        }
    }
}

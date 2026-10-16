//! Worktree cleanup and the Widget, ported from vornd's built-ins to what a
//! package can reach: the extension API's `host.call` and `host.publish`,
//! calls and events in. Written once; the program tier wraps it in stdio
//! JSON-RPC, the Wasm tier in the component's exports, and the bench runs it
//! in-process as the built-in baseline.
//!
//! The API has no `rows()`, so an extension keeps its own copy of what it
//! published ([`Rows`]).

pub mod widget;
pub mod worktrees;

use serde_json::Value;

/// One change to a collection, as `host.publish` takes it.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Upsert(Value),
    Delete(String),
    Clear,
}

/// A failed call, as the API's `ext-error`.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl Error {
    pub fn new(code: &str, message: impl Into<String>) -> Error {
        Error {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

/// What an extension reaches Vorn through.
pub trait Host {
    fn call(&mut self, method: &str, args: Value) -> Result<Value, Error>;
    fn publish(&mut self, collection: &str, ops: Vec<Op>);
}

/// An extension's service.
pub trait Extension: Send {
    /// Once started: publishes what it shows.
    fn init(&mut self, host: &mut dyn Host);
    /// A client's call to one of its actions.
    fn handle(&mut self, host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error>;
    /// A batch of the events it subscribes to, in order, as `(name, data)`.
    fn on_events(&mut self, host: &mut dyn Host, batch: &[(String, Value)]);
}

/// The extension a package id names.
pub fn by_id(id: &str) -> Option<Box<dyn Extension>> {
    match id {
        "vorn.worktrees" => Some(Box::<worktrees::Cleanup>::default()),
        "vorn.widget" => Some(Box::<widget::Widget>::default()),
        _ => None,
    }
}

/// The bench's own actions, the same in every tier: an echo for the bare
/// cost of a call, and a crash, a spin and a memory grab for isolation.
/// `None` for any other action.
pub fn probe(action: &str, args: &Value) -> Option<Result<Value, Error>> {
    match action {
        "bench.echo" => Some(Ok(args.clone())),
        "bench.crash" => panic!("bench.crash: the probe crashes on purpose"),
        "bench.spin" => {
            let mut n: u64 = 0;
            loop {
                n = std::hint::black_box(n.wrapping_add(1));
            }
        }
        "bench.grow" => {
            let mb = args["mb"].as_u64().unwrap_or(256) as usize;
            let mut held: Vec<u8> = vec![0; mb << 20];
            for i in (0..held.len()).step_by(4096) {
                held[i] = 1;
            }
            Some(Ok(Value::from(std::hint::black_box(held).len())))
        }
        _ => None,
    }
}

/// What an extension published, kept so it can read its rows back.
#[derive(Debug, Default)]
pub struct Rows {
    kept: Vec<(String, Vec<(String, Value)>)>,
}

impl Rows {
    /// Applies `ops` here and publishes them; nothing is sent for none.
    pub fn publish(&mut self, host: &mut dyn Host, collection: &str, ops: Vec<Op>) {
        if ops.is_empty() {
            return;
        }
        let rows = match self.kept.iter().position(|(c, _)| c == collection) {
            Some(at) => &mut self.kept[at].1,
            None => {
                self.kept.push((collection.to_owned(), Vec::new()));
                &mut self.kept.last_mut().expect("just pushed").1
            }
        };
        for op in &ops {
            match op {
                Op::Upsert(row) => {
                    let Some(id) = row["id"].as_str() else {
                        continue;
                    };
                    match rows.iter_mut().find(|(k, _)| k == id) {
                        Some(slot) => slot.1 = row.clone(),
                        None => rows.push((id.to_owned(), row.clone())),
                    }
                }
                Op::Delete(id) => rows.retain(|(k, _)| k != id),
                Op::Clear => rows.clear(),
            }
        }
        host.publish(collection, ops);
    }

    /// The rows of `collection`, in the order they came.
    pub fn rows(&self, collection: &str) -> Vec<Value> {
        self.kept
            .iter()
            .find(|(c, _)| c == collection)
            .map(|(_, rows)| rows.iter().map(|(_, r)| r.clone()).collect())
            .unwrap_or_default()
    }
}

/// Argument `key` of `action`, a string.
pub(crate) fn arg<'a>(args: &'a Value, key: &str, action: &str) -> Result<&'a str, Error> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::new("bad-request", format!("{action} takes {key}")))
}

pub(crate) fn unknown(name: &str, action: &str) -> Error {
    Error::new("unknown-action", format!("{name} has no action {action}"))
}

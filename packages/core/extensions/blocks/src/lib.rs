//! Shell blocks: a shell's commands from its OSC 133 marks (what ran, where,
//! how it ended, how long it took), drawn by the client as blocks over the
//! shell's live rows from the `commands` tree, and the history and
//! completions the shell's composer offers.
//!
//! Vorn ships it as a Wasm component (`vorn-extension.toml` beside this
//! crate). A shell's commands and the history are kept in its storage, so a
//! restart of its instance loses nothing a person sees; the completion
//! caches live only as long as the instance.

use std::collections::{BTreeMap, HashMap};

use vorn_extension_sdk::tree::{self, sdk, Command, Log, Surface};
use vorn_extension_sdk::{arg, json, serde_json, Ctx, Error, Event, Extension, Host, Init, Op, Value};

pub mod complete;
pub mod history;
pub mod marks;

use complete::{Completion, Place, Sources};
use history::History;
use marks::Marked;

/// The `session.mark` event: a shell's prompt and command marks.
const SESSION_MARK: &str = "session.mark";
const SESSION_EXITED: &str = "session.exited";
/// The tree of a shell's commands.
pub const COMMANDS: &str = "commands";
/// One row per shell whose commands changed, its id the session's: the
/// client asks again for that shell's tree alone.
pub const SHELLS: &str = "shells";

fn stored(session: &str) -> String {
    format!("shell/{session}")
}

/// The extension: what it has read of storage this instance, and the completion caches.
#[derive(Debug, Default)]
pub struct Blocks {
    shells: HashMap<String, Vec<Command>>,
    history: Option<History>,
    sources: Sources,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl Blocks {
    /// `session`'s commands, read from storage the first time.
    fn commands(&mut self, host: &mut dyn Host, session: &str) -> Result<&mut Vec<Command>, Error> {
        if !self.shells.contains_key(session) {
            let kept = host
                .get(&stored(session))?
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            self.shells.insert(session.to_owned(), kept);
        }
        Ok(self.shells.entry(session.to_owned()).or_default())
    }

    fn history(&mut self, host: &mut dyn Host) -> Result<&mut History, Error> {
        if self.history.is_none() {
            self.history = Some(History::load(host)?);
        }
        Ok(self.history.get_or_insert_with(History::default))
    }

    /// Folds a batch of marks in, then keeps and announces each shell they touched once.
    fn marked(&mut self, host: &mut dyn Host, batch: &[Event]) -> Result<(), Error> {
        let mut touched = BTreeMap::new();
        for e in batch {
            if e.name == SESSION_EXITED {
                let Some(id) = text(&e.data, "sessionId") else {
                    continue;
                };
                self.shells.remove(id);
                touched.insert(id.to_owned(), false);
                continue;
            }
            let Some(m) = Marked::read(&e.data).filter(|_| e.name == SESSION_MARK) else {
                continue;
            };
            let commands = self.commands(host, &m.session)?;
            marks::fold(commands, m.mark, m.at, m.cwd.as_deref());
            touched.insert(m.session, true);
        }
        let mut ops = Vec::new();
        for (id, live) in touched {
            match self.shells.get(&id).filter(|_| live) {
                Some(commands) => {
                    let v = serde_json::to_value(commands)
                        .map_err(|e| Error::new("failed", e.to_string()))?;
                    host.set(&stored(&id), &json!(v))?;
                    ops.push(Op::Upsert(json!({ "id": id, "n": commands.len() })));
                }
                None => {
                    host.set(&stored(&id), &Value::Null)?;
                    ops.push(Op::Delete(id));
                }
            }
        }
        if !ops.is_empty() {
            host.publish(SHELLS, ops);
        }
        Ok(())
    }

    fn act(&mut self, host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        match action {
            "history" => {
                let typed = args.get("text").and_then(Value::as_str).unwrap_or("");
                let project = text(args, "project");
                let h = self.history(host)?;
                let rows: Vec<Value> = h
                    .search(typed, project)
                    .into_iter()
                    .map(|e| json!({ "text": e.text, "timestamp": e.timestamp }))
                    .collect();
                Ok(json!({ "rows": rows, "ghost": h.ghost(typed, project) }))
            }
            "complete" => {
                let typed = args.get("text").and_then(Value::as_str).unwrap_or("");
                let place = Place {
                    cwd: text(args, "cwd"),
                    repo: text(args, "repo"),
                };
                let found = complete::complete_with(host, &mut self.sources, typed, &place);
                Ok(json!({
                    "completions": found.iter().map(|c| completion(typed, c)).collect::<Vec<_>>(),
                    "tab": complete::tab(typed, &found),
                }))
            }
            "record" => {
                let line = arg(args, "text", action)?;
                let project = text(args, "project");
                self.history(host)?.record(host, line, project, now_ms())?;
                Ok(Value::Null)
            }
            "import" => {
                let more = history::entries(args.get("entries").unwrap_or(&Value::Null));
                let n = self.history(host)?.import(host, more)?;
                Ok(json!({ "imported": n }))
            }
            other => Err(Error::unknown(other)),
        }
    }
}

/// A completion as the composer's menu draws and takes it.
fn completion(typed: &str, c: &Completion) -> Value {
    json!({
        "label": c.label,
        "detail": c.detail,
        "icon": c.kind.icon(c.continues),
        "continues": c.continues,
        "result": complete::apply(typed, c),
    })
}

impl Extension for Blocks {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Blocks::default())
    }

    fn handle(&mut self, host: &mut dyn Host, method: &str, args: Value, _: &Ctx) -> Result<Value, Error> {
        self.act(host, method, &args)
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: Vec<Event>) {
        if let Err(e) = self.marked(host, &batch) {
            host.log(
                vorn_extension_sdk::Level::Warn,
                &format!("marks not kept: {}", e.message),
            );
        }
    }

    fn render(&mut self, host: &mut dyn Host, point: &str, surface: Surface) -> Result<tree::Node, Error> {
        if point != COMMANDS {
            return Err(Error::new("unknown-action", format!("nothing draws {point}")));
        }
        let commands = match surface.scope.as_deref() {
            Some(session) => self.commands(host, session)?.clone(),
            None => Vec::new(),
        };
        Ok(sdk::log(Log { commands }))
    }
}

vorn_extension_sdk::export!(Blocks);

#[cfg(test)]
mod tests;

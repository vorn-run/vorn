//! Tasks, the built-in, as a package's service: the program and Wasm tiers
//! run this. The built-in's own `calls.rs`, `triggers.rs` and `tools.rs`
//! are compiled here unchanged; [`builtin`] is its `builtin.rs` ported to
//! the extension API, and the shim stands in for the store and vorn-work.

// Those files' tests are E6's, run against the real store in its crate.
#![cfg(not(test))]

// `calls.rs` and `triggers.rs` name `vorn_work::js`, which the shim has too.
extern crate vorn_store as vorn_work;

pub mod builtin;
// E6's own file; its crate formats it.
#[rustfmt::skip]
#[path = "../../../../crates/tasks/src/calls.rs"]
pub mod calls;
// E6's own file; its crate formats it.
#[rustfmt::skip]
#[path = "../../../../crates/tasks/src/tools.rs"]
pub mod tools;
// E6's own file; its crate formats it.
#[rustfmt::skip]
#[path = "../../../../crates/tasks/src/triggers.rs"]
pub mod triggers;

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use builtin::{Answer, Tasks};
use ext_tiers_logic::{Error, Extension, Host};
use serde_json::Value;

/// The id the built-in goes by.
pub const ID: &str = "vorn.tasks";

/// Tasks served over `host`, which it keeps: its tools call it from within their own futures.
pub fn by_id(id: &str, host: Box<dyn Host + Send>) -> Option<Box<dyn Extension>> {
    (id == ID).then(|| {
        Box::new(Service {
            tasks: Arc::new(Tasks::new(host)),
        }) as Box<dyn Extension>
    })
}

/// Runs a future none of whose calls wait: every host call answers before it returns.
fn now<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(out) => out,
        Poll::Pending => unreachable!("a guest's host calls answer before they return"),
    }
}

struct Service {
    tasks: Arc<Tasks>,
}

impl Extension for Service {
    fn init(&mut self, _: &mut dyn Host) {
        self.tasks.badges();
    }

    fn handle(&mut self, _: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        if let Some(tool) = action.strip_prefix("mcp.") {
            let Some(result) = now(tools::call(&self.tasks, tool, args.clone())) else {
                return Err(Error::new("unknown-action", format!("no tool {tool}")));
            };
            let text = result
                .pointer("/content/0/text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if result.get("isError") == Some(&Value::Bool(true)) {
                return Err(Error::new("failed", text));
            }
            return Ok(Value::String(text));
        }
        match self.tasks.answer_now(action, args.clone()) {
            Answer::Result(v) => Ok(v),
            Answer::Void => Ok(Value::Null),
            Answer::Error(message) => Err(Error::new("failed", message)),
            Answer::Unanswered => Err(Error::new(
                "unknown-action",
                format!("{ID} has no action {action}"),
            )),
        }
    }

    fn on_events(&mut self, _: &mut dyn Host, batch: &[(String, Value)]) {
        for (name, data) in batch {
            let text = |k: &str| data.get(k).and_then(Value::as_str);
            if let ("agent.conversation", Some(terminal), Some(conversation)) =
                (name.as_str(), text("sessionId"), text("conversationId"))
            {
                self.tasks.keep_conversation(terminal, conversation);
            }
        }
    }
}

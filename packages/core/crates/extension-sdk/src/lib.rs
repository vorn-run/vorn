//! Write a Vorn extension in Rust, compiled to a Wasm component.
//!
//! An extension is a type that implements [`Extension`]. Vorn calls it with
//! a [`Host`]: every call back into Vorn goes through it, held to what the
//! manifest declares. [`export!`] makes the type the component's guest:
//!
//! ```ignore
//! use vorn_extension_sdk::{export, tree::sdk::*, Ctx, Error, Extension, Host, Init, Value};
//!
//! struct Hello;
//!
//! impl Extension for Hello {
//!     fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
//!         Ok(Hello)
//!     }
//!     fn handle(&mut self, host: &mut dyn Host, method: &str, _: Value, _: &Ctx) -> Result<Value, Error> {
//!         match method {
//!             "sessions" => host.call("sessions.list", Value::Null),
//!             other => Err(Error::unknown(other)),
//!         }
//!     }
//! }
//!
//! export!(Hello);
//! ```
//!
//! Build it with `cargo build --release --target wasm32-wasip2` and name the
//! `.wasm` in the manifest: `[service] component = "target/.../hello.wasm"`.
//! `vorn ext new --wasm` writes such a package, and `vorn ext link --watch`
//! rebuilds and reloads it on every save.
//!
//! The same [`Extension`] runs off Wasm against [`testing::FakeHost`], so its
//! logic is tested with plain `cargo test`.

use std::fmt;

use serde::{Deserialize, Serialize};

pub use serde_json::{self, json, Value};
/// The element tree a [`Extension::render`] answers, and its builder in [`tree::sdk`].
pub use vorn_tree as tree;

#[doc(hidden)]
pub mod bindings;
#[doc(hidden)]
pub mod guest;
pub mod testing;

/// A call that failed: a code a caller can match, and words a person reads.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// The error for an action the extension does not have.
    pub fn unknown(action: &str) -> Error {
        Error::new("unknown-action", format!("no action {action}"))
    }

    /// The error for args that do not read.
    pub fn bad_args(message: impl Into<String>) -> Error {
        Error::new("bad-request", message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// What the extension is started with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Init {
    /// Its id, from the manifest.
    pub id: String,
    /// Its version, from the manifest.
    pub version: String,
    /// The Vorn version running it.
    pub vorn: String,
}

/// Who is calling, and about which project.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ctx {
    /// `client`, `mcp`, `workflow`, ...
    pub caller: String,
    pub project: Option<String>,
}

/// One event it subscribes to.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: String,
    pub data: Value,
}

/// One change to a collection of rows; a row's key is its `id`.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Upsert(Value),
    Delete(String),
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

/// A session to start, as the app and the MCP tools start one.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionStart {
    /// The folder of the project it runs in.
    pub project: String,
    /// An agent type as `agents.list` names it; none starts a shell.
    pub agent: Option<String>,
    /// What an agent is asked first.
    pub prompt: Option<String>,
    /// Starts it in a new worktree of the project.
    pub worktree: bool,
    /// The new worktree's branch.
    pub branch: Option<String>,
    /// The card's name.
    pub name: Option<String>,
    /// An agent run with no terminal.
    pub headless: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// An HTTPS request `fetch` sends: only to a host the manifest declares.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<Header>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<Header>,
    pub body: String,
}

/// One statement of a [`Host::transaction`], with its `?` params.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Value>,
}

impl Statement {
    pub fn new(sql: impl Into<String>, params: impl IntoIterator<Item = Value>) -> Statement {
        Statement {
            sql: sql.into(),
            params: params.into_iter().collect(),
        }
    }
}

/// A file in the extension's own store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub name: String,
    pub size: u64,
    /// When it was last written, in milliseconds since the Unix epoch.
    pub modified: u64,
}

fn unsupported(what: &str) -> Error {
    Error::new("unsupported", format!("this host does not answer {what}"))
}

/// What Vorn answers an extension: every import of the extension world.
/// Each call is held to the permissions the manifest declares.
///
/// In a component it is Vorn itself; in a test, [`testing::FakeHost`] or
/// your own, which needs only the methods the extension uses.
pub trait Host {
    /// A core method, such as `sessions.list`, under the permission it needs.
    fn call(&mut self, method: &str, args: Value) -> Result<Value, Error>;
    /// Changes the rows of a collection its templates or tree draw.
    fn publish(&mut self, collection: &str, ops: Vec<Op>);
    fn log(&mut self, _level: Level, _message: &str) {}
    /// The rows it published to `collection`, in the order they came.
    fn rows(&mut self, _collection: &str) -> Result<Vec<Value>, Error> {
        Err(unsupported("host.rows"))
    }
    /// Starts a session as the app does, under `sessions.launch`; answers its id.
    fn start_session(&mut self, _req: &SessionStart) -> Result<String, Error> {
        Err(unsupported("host.start-session"))
    }
    /// HTTPS to a declared host, with the connection's credential added by Vorn.
    fn fetch(&mut self, _req: &Request, _connection: Option<&str>) -> Result<Response, Error> {
        Err(unsupported("net.fetch"))
    }
    /// The prefix its tables are named with.
    fn prefix(&mut self) -> Result<String, Error> {
        Err(unsupported("storage.prefix"))
    }
    /// The rows a query on its own tables answers, each an object keyed by column.
    fn query(&mut self, _sql: &str, _params: &[Value]) -> Result<Vec<Value>, Error> {
        Err(unsupported("storage.query"))
    }
    /// Runs one statement on its own tables; answers how many rows it changed.
    fn exec(&mut self, _sql: &str, _params: &[Value]) -> Result<u64, Error> {
        Err(unsupported("storage.exec"))
    }
    /// Statements run as one transaction: each one's rows, or how many it changed.
    fn transaction(&mut self, _statements: &[Statement]) -> Result<Vec<Value>, Error> {
        Err(unsupported("storage.transaction"))
    }
    /// The value kept under `key`.
    fn get(&mut self, _key: &str) -> Result<Option<Value>, Error> {
        Err(unsupported("storage.get"))
    }
    /// Keeps `value` under `key`; null removes it.
    fn set(&mut self, _key: &str, _value: &Value) -> Result<(), Error> {
        Err(unsupported("storage.set"))
    }
    /// Writes its own file `name`, whole or not at all.
    fn put_file(&mut self, _name: &str, _data: &[u8]) -> Result<(), Error> {
        Err(unsupported("blobs.put"))
    }
    /// Its own file `name`, or none.
    fn file(&mut self, _name: &str) -> Result<Option<Vec<u8>>, Error> {
        Err(unsupported("blobs.get"))
    }
    /// Its own files whose names start with `prefix`, by name.
    fn files(&mut self, _prefix: &str) -> Result<Vec<Blob>, Error> {
        Err(unsupported("blobs.list"))
    }
    /// Removes its own file `name`; answers whether there was one.
    fn delete_file(&mut self, _name: &str) -> Result<bool, Error> {
        Err(unsupported("blobs.delete"))
    }
}

/// An extension: what it answers, and the events it is woken for.
///
/// One instance lives as long as its component does. Vorn may stop an idle
/// one and start it again, so state a person sees belongs in published rows
/// ([`Host::rows`] reads them back), storage or files, not only in `self`.
pub trait Extension: Sized + 'static {
    /// Starts it; an error means it does not run.
    fn init(init: &Init, host: &mut dyn Host) -> Result<Self, Error>;

    /// A call to one of its actions: a template's button, a client, an MCP
    /// tool (`mcp.<name>`), a workflow node.
    fn handle(
        &mut self,
        _host: &mut dyn Host,
        method: &str,
        _args: Value,
        _ctx: &Ctx,
    ) -> Result<Value, Error> {
        Err(Error::unknown(method))
    }

    /// A batch of the events its manifest subscribes to, in order.
    fn on_events(&mut self, _host: &mut dyn Host, _batch: Vec<Event>) {}

    /// The tree for `point`, a `tree = true` contribution of its manifest.
    fn render(
        &mut self,
        _host: &mut dyn Host,
        point: &str,
        _surface: tree::Surface,
    ) -> Result<tree::Node, Error> {
        Err(Error::new(
            "unknown-action",
            format!("nothing draws {point}"),
        ))
    }

    /// Something happened on `point`'s tree: answers `{toast?, open?, patch?}`.
    fn event(
        &mut self,
        _host: &mut dyn Host,
        point: &str,
        event: tree::Event,
    ) -> Result<Value, Error> {
        Err(Error::new(
            "unknown-action",
            format!("{point} has no {}", event.id),
        ))
    }
}

#[derive(Deserialize)]
struct RenderArgs {
    #[serde(default)]
    point: String,
    #[serde(default)]
    surface: tree::Surface,
}

#[derive(Deserialize)]
struct EventArgs {
    #[serde(default)]
    point: String,
    event: tree::Event,
}

/// Routes a `guest.handle` to `ext`: `ui.render` and `ui.event` to its tree,
/// anything else to [`Extension::handle`]. What the component runs, and what
/// a test calls to drive the extension the same way.
pub fn dispatch<E: Extension>(
    ext: &mut E,
    host: &mut dyn Host,
    method: &str,
    args: Value,
    ctx: &Ctx,
) -> Result<Value, Error> {
    match method {
        tree::RENDER => {
            let a: RenderArgs = serde_json::from_value(args)
                .map_err(|e| Error::bad_args(format!("{}: {e}", tree::RENDER)))?;
            let node = ext.render(host, &a.point, a.surface)?;
            tree::check(&node).map_err(|errors| {
                Error::new(
                    "failed",
                    format!("the tree for {} cannot be drawn: {}", a.point, errors.join("; ")),
                )
            })?;
            serde_json::to_value(tree::Rendered::new(node))
                .map_err(|e| Error::new("failed", e.to_string()))
        }
        tree::EVENT => {
            let a: EventArgs = serde_json::from_value(args)
                .map_err(|e| Error::bad_args(format!("{}: {e}", tree::EVENT)))?;
            ext.event(host, &a.point, a.event)
        }
        _ => ext.handle(host, method, args, ctx),
    }
}

/// A string argument, or the error that says `action` takes it.
pub fn arg<'a>(args: &'a Value, key: &str, action: &str) -> Result<&'a str, Error> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::bad_args(format!("{action} takes {key}")))
}

/// Makes `$ty`, an [`Extension`], the guest of this component.
#[macro_export]
macro_rules! export {
    ($ty:ty) => {
        #[doc(hidden)]
        pub struct __VornGuest;

        impl $crate::bindings::exports::vorn::extension::guest::Guest for __VornGuest {
            fn init(
                ctx: $crate::bindings::vorn::extension::types::InitCtx,
            ) -> ::core::result::Result<(), $crate::bindings::vorn::extension::types::ExtError> {
                $crate::guest::init::<$ty>(ctx)
            }

            fn handle(
                method: ::std::string::String,
                args: ::std::string::String,
                ctx: $crate::bindings::vorn::extension::types::CallCtx,
            ) -> ::core::result::Result<
                ::std::string::String,
                $crate::bindings::vorn::extension::types::ExtError,
            > {
                $crate::guest::handle::<$ty>(method, args, ctx)
            }

            fn on_event(batch: ::std::vec::Vec<$crate::bindings::vorn::extension::types::Event>) {
                $crate::guest::on_event::<$ty>(batch)
            }
        }

        $crate::bindings::__export_extension!(__VornGuest with_types_in $crate::bindings);
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHost;
    use crate::tree::sdk::*;

    struct Page {
        clicks: u32,
    }

    impl Extension for Page {
        fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
            Ok(Page { clicks: 0 })
        }

        fn render(&mut self, _: &mut dyn Host, point: &str, s: tree::Surface) -> Result<tree::Node, Error> {
            let label = if s.cells { "cells" } else { "pixels" };
            Ok(col([text(format!("{point} on {label}")), button("go", "Go")]))
        }

        fn event(&mut self, host: &mut dyn Host, _: &str, e: tree::Event) -> Result<Value, Error> {
            self.clicks += 1;
            host.publish("clicks", vec![Op::Upsert(json!({ "id": e.id, "n": self.clicks }))]);
            Ok(json!({ "toast": "Clicked" }))
        }
    }

    #[test]
    fn renders_and_answers_its_tree_through_dispatch() {
        let mut host = FakeHost::default();
        let mut page = Page::init(&Init::default(), &mut host).unwrap();
        let ctx = Ctx::default();
        let args = json!({ "point": "settings/p", "surface": { "cells": true } });
        let drawn = dispatch(&mut page, &mut host, tree::RENDER, args, &ctx).unwrap();
        let read = tree::read(&drawn).expect("a tree the host draws");
        assert!(serde_json::to_string(&read.tree).unwrap().contains("settings/p on cells"));
        let click = json!({ "point": "settings/p", "event": { "id": "go", "kind": "click" } });
        let done = dispatch(&mut page, &mut host, tree::EVENT, click, &ctx).unwrap();
        assert_eq!(done, json!({ "toast": "Clicked" }));
        assert_eq!(host.rows("clicks").unwrap(), [json!({ "id": "go", "n": 1 })]);
        let err = dispatch(&mut page, &mut host, "other", Value::Null, &ctx).unwrap_err();
        assert_eq!(err.code, "unknown-action");
        let bad = dispatch(&mut page, &mut host, tree::EVENT, json!({}), &ctx).unwrap_err();
        assert_eq!(bad.code, "bad-request");
    }

    #[test]
    fn an_arg_is_a_string_that_is_there() {
        let args = json!({ "id": "a", "empty": "", "n": 1 });
        assert_eq!(arg(&args, "id", "go").unwrap(), "a");
        for key in ["empty", "n", "missing"] {
            assert_eq!(arg(&args, key, "go").unwrap_err().message, format!("go takes {key}"));
        }
    }
}

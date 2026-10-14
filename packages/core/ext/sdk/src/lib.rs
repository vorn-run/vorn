//! Write a Vorn extension in Rust.
//!
//! An extension implements [`Extension`] and calls back into Vorn only
//! through [`host`] and [`net`]. The transport is chosen when it is built:
//! for `wasm32` the calls are the `vorn:extension` WIT's imports and exports
//! (the safe tier); natively they are the same calls as JSON-RPC over stdio
//! (`cargo run`, the quick tier), so the code does not change between
//! developing and packing.
//!
//! ```ignore
//! struct Hello;
//! impl vorn_extension::Extension for Hello {
//!     fn handle(method: &str, _: Value, _: &Ctx) -> Result<Value, Error> {
//!         Ok(json!([{ "id": "1", "name": method }]))
//!     }
//! }
//! vorn_extension::export!(Hello);
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};
pub use serde_json::{self, json, Value};

/// A failed call: a stable code and words a person reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

/// Who called, and for which project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ctx {
    pub project: String,
    pub caller: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<String>,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Request {
        Request {
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    /// The body as JSON, or an error naming the status when it is not a success.
    pub fn json(&self) -> Result<Value, Error> {
        if !(200..300).contains(&self.status) {
            return Err(Error::new(
                "http",
                format!(
                    "HTTP {}: {}",
                    self.status,
                    self.body.chars().take(200).collect::<String>()
                ),
            ));
        }
        serde_json::from_str(&self.body)
            .map_err(|e| Error::new("bad-output", format!("not JSON: {e}")))
    }
}

/// An extension: one entry point for every method its manifest names.
pub trait Extension {
    fn handle(method: &str, args: Value, ctx: &Ctx) -> Result<Value, Error>;
}

/// Calls back into Vorn.
pub mod host {
    use super::{Error, Value};

    /// A core method (`git.branch`), allowed by the manifest's permissions.
    pub fn call(method: &str, args: &Value) -> Result<Value, Error> {
        crate::transport::call(method, args)
    }

    pub fn log(message: &str) {
        crate::transport::log(message)
    }
}

/// HTTP, made by vornd: only to the manifest's hosts, with its credential added there.
pub mod net {
    use super::{Error, Request, Response};

    pub fn fetch(req: &Request) -> Result<Response, Error> {
        crate::transport::fetch(req)
    }
}

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        world: "extension",
        path: "../wit",
        pub_export_macro: true,
        default_bindings_module: "vorn_extension::bindings",
    });
}

#[cfg(target_arch = "wasm32")]
mod transport {
    use super::{Error, Request, Response, Value};
    use crate::bindings::vorn::extension::{host, net, types};

    fn err(e: types::ExtError) -> Error {
        Error {
            code: e.code,
            message: e.message,
        }
    }

    pub fn call(method: &str, args: &Value) -> Result<Value, Error> {
        let out = host::call(method, &args.to_string()).map_err(err)?;
        serde_json::from_str(&out).map_err(|e| Error::new("bad-output", e.to_string()))
    }

    pub fn log(message: &str) {
        host::log(types::Level::Info, message);
    }

    pub fn fetch(req: &Request) -> Result<Response, Error> {
        let r = types::Request {
            method: req.method.clone(),
            url: req.url.clone(),
            headers: req
                .headers
                .iter()
                .map(|(name, value)| types::Header {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
            body: req.body.clone(),
        };
        let res = net::fetch(&r, None).map_err(err)?;
        Ok(Response {
            status: res.status,
            headers: res.headers.into_iter().map(|h| (h.name, h.value)).collect(),
            body: res.body,
        })
    }
}

/// What [`export!`] expands to on wasm32; not for direct use.
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub fn __handle<T: Extension>(
    method: String,
    args: String,
    ctx: bindings::vorn::extension::types::CallCtx,
) -> Result<String, bindings::vorn::extension::types::ExtError> {
    let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
    let ctx = Ctx {
        project: ctx.project,
        caller: ctx.caller,
    };
    T::handle(&method, args, &ctx)
        .map(|v| v.to_string())
        .map_err(|e| bindings::vorn::extension::types::ExtError {
            code: e.code,
            message: e.message,
        })
}

/// Exports an [`Extension`] as the component's `guest` interface.
#[cfg(target_arch = "wasm32")]
#[macro_export]
macro_rules! export {
    ($t:ty) => {
        struct __VornGuest;
        impl $crate::bindings::exports::vorn::extension::guest::Guest for __VornGuest {
            fn handle(
                method: String,
                args: String,
                ctx: $crate::bindings::vorn::extension::types::CallCtx,
            ) -> Result<String, $crate::bindings::vorn::extension::types::ExtError> {
                $crate::__handle::<$t>(method, args, ctx)
            }
        }
        $crate::bindings::export!(__VornGuest with_types_in $crate::bindings);
    };
}

/// Natively an extension is a program: `fn main() { vorn_extension::serve::<T>() }`.
#[cfg(not(target_arch = "wasm32"))]
#[macro_export]
macro_rules! export {
    ($t:ty) => {};
}

#[cfg(not(target_arch = "wasm32"))]
mod transport {
    //! The WIT's calls as JSON-RPC 2.0 over stdio, a message per line.
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{json, Error, Request, Response, Value};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    pub fn send(msg: &Value) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{msg}");
        let _ = out.flush();
    }

    pub fn read() -> Option<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            if std::io::stdin().lock().read_line(&mut line).ok()? == 0 {
                return None;
            }
            if let Ok(v) = serde_json::from_str(&line) {
                return Some(v);
            }
        }
    }

    fn request(method: &str, params: Value) -> Result<Value, Error> {
        let id = format!("g{}", NEXT.fetch_add(1, Ordering::Relaxed));
        send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let msg = read().ok_or_else(|| Error::new("exited", "Vorn closed stdin"))?;
            if msg["id"] == json!(id) {
                return match msg.get("error") {
                    Some(e) => Err(Error::new(
                        e["data"]["code"].as_str().unwrap_or("failed"),
                        e["message"].as_str().unwrap_or("failed"),
                    )),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                };
            }
        }
    }

    pub fn call(method: &str, args: &Value) -> Result<Value, Error> {
        request(
            "host.call",
            json!({ "method": method, "args": args.to_string() }),
        )
    }

    pub fn log(message: &str) {
        send(
            &json!({ "jsonrpc": "2.0", "method": "host.log", "params": { "level": "info", "message": message } }),
        );
    }

    pub fn fetch(req: &Request) -> Result<Response, Error> {
        let v = request("net.fetch", json!({ "req": req, "connection": null }))?;
        serde_json::from_value(v).map_err(|e| Error::new("bad-output", e.to_string()))
    }
}

/// Serves `T` over stdio until Vorn closes stdin: the native transport.
#[cfg(not(target_arch = "wasm32"))]
pub fn serve<T: Extension>() {
    while let Some(msg) = transport::read() {
        if msg["method"] != "guest.handle" {
            continue;
        }
        let p = &msg["params"];
        let ctx: Ctx = serde_json::from_value(p["ctx"].clone()).unwrap_or_default();
        let answer = T::handle(p["method"].as_str().unwrap_or(""), p["args"].clone(), &ctx);
        let reply = match answer {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": msg["id"], "result": result }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": msg["id"],
                "error": { "code": -32000, "message": e.message, "data": { "code": e.code } } }),
        };
        transport::send(&reply);
    }
}

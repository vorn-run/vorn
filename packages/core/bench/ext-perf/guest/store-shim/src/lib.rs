//! The store an extension reaches through vornd's bridge.
//!
//! It keeps `vorn-store`'s calling shape (`call(name, args)`), so
//! `vorn-work`'s Tasks code compiles unchanged against it; every call is
//! sent as JSON text and its answer read back, which is the cost a
//! component pays per storage call.

use std::fmt;

use serde_json::Value;

/// A storage call that failed, with the bridge's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Answers one named call: JSON arguments in, JSON answer out.
pub type Bridge = Box<dyn FnMut(&str, &str) -> std::result::Result<String, String>>;

/// The store behind a bridge.
pub struct Store {
    bridge: Bridge,
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Store(bridge)")
    }
}

impl Store {
    pub fn new(
        bridge: impl FnMut(&str, &str) -> std::result::Result<String, String> + 'static,
    ) -> Store {
        Store {
            bridge: Box::new(bridge),
        }
    }

    /// `vorn_store::Store::call` across the bridge.
    pub fn call(&mut self, call: &str, args: Value) -> Result<Value> {
        let answer = (self.bridge)(call, &args.to_string()).map_err(Error)?;
        serde_json::from_str(&answer)
            .map_err(|e| Error(format!("{call} answered with bad JSON: {e}")))
    }

    /// A real store in memory behind the bridge, for tests.
    #[cfg(feature = "real")]
    pub fn open_in_memory(options: StoreOptions) -> Result<Store> {
        let mut real =
            vorn_store::Store::open_in_memory(options).map_err(|e| Error(e.to_string()))?;
        Ok(Store::new(move |call, args| {
            let args: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
            real.call(call, args)
                .map(|v| v.to_string())
                .map_err(|e| e.to_string())
        }))
    }
}

#[cfg(feature = "real")]
pub use vorn_store::StoreOptions;

//! [`Store::call`]: a call by its TypeScript name, with JSON arguments.

use serde_json::Value;

use crate::{Error, Result, Store};

pub(crate) fn call(_store: &mut Store, call: &str, _args: Value) -> Result<Value> {
    Err(Error::UnknownCall(call.to_owned()))
}

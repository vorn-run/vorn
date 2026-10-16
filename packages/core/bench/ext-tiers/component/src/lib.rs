//! An extension served as a Wasm component of the `extension` world in
//! `wit/extension.wit`, for `wasm32-wasip2`. It serves whichever extension
//! `init` names.

use std::sync::{Mutex, PoisonError};

use ext_tiers_logic::{probe, Error, Extension, Host, Op};
use serde_json::Value;

wit_bindgen::generate!({
    world: "extension",
    path: "../../../crates/extensions/wit/extension.wit",
});

use exports::vorn::extension::guest::{CallCtx, Event, Guest, InitCtx};
use vorn::extension::host;
use vorn::extension::types::{ExtError, RowOp};

static EXT: Mutex<Option<Box<dyn Extension>>> = Mutex::new(None);

/// The host the component imports.
struct Imports;

impl Host for Imports {
    fn call(&mut self, method: &str, args: Value) -> Result<Value, Error> {
        let answer = host::call(method, &args.to_string()).map_err(|e| Error {
            code: e.code,
            message: e.message,
        })?;
        Ok(serde_json::from_str(&answer).unwrap_or(Value::Null))
    }

    fn publish(&mut self, collection: &str, ops: Vec<Op>) {
        let ops: Vec<RowOp> = ops
            .into_iter()
            .map(|op| match op {
                Op::Upsert(row) => RowOp::Upsert(row.to_string()),
                Op::Delete(id) => RowOp::Delete(id),
                Op::Clear => RowOp::Clear,
            })
            .collect();
        host::publish(collection, &ops);
    }
}

fn ext_error(e: Error) -> ExtError {
    ExtError {
        code: e.code,
        message: e.message,
    }
}

struct Component;

impl Guest for Component {
    fn init(ctx: InitCtx) -> Result<(), ExtError> {
        let mut ext = ext_tiers_logic::by_id(&ctx.id)
            .ok_or_else(|| ext_error(Error::new("unknown", format!("no extension {}", ctx.id))))?;
        ext.init(&mut Imports);
        *EXT.lock().unwrap_or_else(PoisonError::into_inner) = Some(ext);
        Ok(())
    }

    fn handle(method: String, args: String, _ctx: CallCtx) -> Result<String, ExtError> {
        let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
        let answer = match probe(&method, &args) {
            Some(probed) => probed,
            None => match EXT.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                Some(ext) => ext.handle(&mut Imports, &method, &args),
                None => Err(Error::new("start", "init came first")),
            },
        };
        answer.map(|v| v.to_string()).map_err(ext_error)
    }

    fn on_event(batch: Vec<Event>) {
        let batch: Vec<(String, Value)> = batch
            .into_iter()
            .map(|e| (e.name, serde_json::from_str(&e.data).unwrap_or(Value::Null)))
            .collect();
        if let Some(ext) = EXT.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            ext.on_events(&mut Imports, &batch);
        }
    }
}

export!(Component);

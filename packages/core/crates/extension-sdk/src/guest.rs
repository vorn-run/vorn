//! The component's side of the world: [`Vorn`], the [`Host`] that is Vorn
//! itself, and what [`crate::export!`]'s guest calls, which keeps the one
//! instance of the extension.

use std::any::Any;
use std::cell::RefCell;

use serde_json::Value;

use crate::bindings::vorn::extension::{blobs, host, net, storage, types};
use crate::{
    Blob, Ctx, Error, Event, Extension, Header, Host, Init, Level, Op, Request, Response,
    SessionStart, Statement,
};

thread_local! {
    /// The extension, once `init` made it; a component runs on one thread.
    static EXT: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None) };
}

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn from_wit(e: types::ExtError) -> Error {
    Error {
        code: e.code,
        message: e.message,
    }
}

fn to_wit(e: Error) -> types::ExtError {
    types::ExtError {
        code: e.code,
        message: e.message,
    }
}

fn rows(texts: Vec<String>) -> Vec<Value> {
    texts.iter().map(|t| parsed(t)).collect()
}

/// Vorn, through the component's imports.
#[derive(Debug, Default, Clone, Copy)]
pub struct Vorn;

impl Host for Vorn {
    fn call(&mut self, method: &str, args: Value) -> Result<Value, Error> {
        host::call(method, &args.to_string())
            .map(|t| parsed(&t))
            .map_err(from_wit)
    }

    fn publish(&mut self, collection: &str, ops: Vec<Op>) {
        let ops: Vec<types::RowOp> = ops
            .into_iter()
            .map(|op| match op {
                Op::Upsert(row) => types::RowOp::Upsert(row.to_string()),
                Op::Delete(id) => types::RowOp::Delete(id),
                Op::Clear => types::RowOp::Clear,
            })
            .collect();
        host::publish(collection, &ops);
    }

    fn log(&mut self, level: Level, message: &str) {
        let level = match level {
            Level::Debug => types::Level::Debug,
            Level::Info => types::Level::Info,
            Level::Warn => types::Level::Warn,
            Level::Error => types::Level::Error,
        };
        host::log(level, message);
    }

    fn rows(&mut self, collection: &str) -> Result<Vec<Value>, Error> {
        host::rows(collection).map(rows).map_err(from_wit)
    }

    fn start_session(&mut self, req: &SessionStart) -> Result<String, Error> {
        let req = types::SessionStart {
            project: req.project.clone(),
            agent: req.agent.clone(),
            prompt: req.prompt.clone(),
            worktree: req.worktree,
            branch: req.branch.clone(),
            name: req.name.clone(),
            headless: req.headless,
        };
        host::start_session(&req).map_err(from_wit)
    }

    fn fetch(&mut self, req: &Request, connection: Option<&str>) -> Result<Response, Error> {
        let wit_req = types::Request {
            method: req.method.clone(),
            url: req.url.clone(),
            headers: req
                .headers
                .iter()
                .map(|h| types::Header {
                    name: h.name.clone(),
                    value: h.value.clone(),
                })
                .collect(),
            body: req.body.clone(),
        };
        let r = net::fetch(&wit_req, connection).map_err(from_wit)?;
        Ok(Response {
            status: r.status,
            headers: r
                .headers
                .into_iter()
                .map(|h| Header {
                    name: h.name,
                    value: h.value,
                })
                .collect(),
            body: r.body,
        })
    }

    fn prefix(&mut self) -> Result<String, Error> {
        storage::prefix().map_err(from_wit)
    }

    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Value>, Error> {
        let params: Vec<String> = params.iter().map(Value::to_string).collect();
        storage::query(sql, &params).map(rows).map_err(from_wit)
    }

    fn exec(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
        let params: Vec<String> = params.iter().map(Value::to_string).collect();
        storage::exec(sql, &params).map_err(from_wit)
    }

    fn transaction(&mut self, statements: &[Statement]) -> Result<Vec<Value>, Error> {
        let statements: Vec<types::Statement> = statements
            .iter()
            .map(|s| types::Statement {
                sql: s.sql.clone(),
                params: s.params.iter().map(Value::to_string).collect(),
            })
            .collect();
        storage::transaction(&statements)
            .map(rows)
            .map_err(from_wit)
    }

    fn get(&mut self, key: &str) -> Result<Option<Value>, Error> {
        storage::get(key)
            .map(|v| v.map(|t| parsed(&t)))
            .map_err(from_wit)
    }

    fn set(&mut self, key: &str, value: &Value) -> Result<(), Error> {
        storage::set(key, &value.to_string()).map_err(from_wit)
    }

    fn put_file(&mut self, name: &str, data: &[u8]) -> Result<(), Error> {
        blobs::put(name, data).map_err(from_wit)
    }

    fn file(&mut self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        blobs::get(name).map_err(from_wit)
    }

    fn files(&mut self, prefix: &str) -> Result<Vec<Blob>, Error> {
        blobs::list(prefix)
            .map(|l| {
                l.into_iter()
                    .map(|b| Blob {
                        name: b.name,
                        size: b.size,
                        modified: b.modified,
                    })
                    .collect()
            })
            .map_err(from_wit)
    }

    fn delete_file(&mut self, name: &str) -> Result<bool, Error> {
        blobs::delete(name).map_err(from_wit)
    }
}

/// Runs `f` on the extension, or says it has not started.
fn with<E: Extension, T>(f: impl FnOnce(&mut E) -> Result<T, Error>) -> Result<T, Error> {
    EXT.with(|cell| {
        let mut held = cell.borrow_mut();
        match held.as_mut().and_then(|b| b.downcast_mut::<E>()) {
            Some(ext) => f(ext),
            None => Err(Error::new("start", "the extension has not started")),
        }
    })
}

pub fn init<E: Extension>(ctx: types::InitCtx) -> Result<(), types::ExtError> {
    let init = Init {
        id: ctx.id,
        version: ctx.version,
        vorn: ctx.vorn,
    };
    let ext = E::init(&init, &mut Vorn).map_err(to_wit)?;
    EXT.with(|cell| *cell.borrow_mut() = Some(Box::new(ext)));
    Ok(())
}

pub fn handle<E: Extension>(
    method: String,
    args: String,
    ctx: types::CallCtx,
) -> Result<String, types::ExtError> {
    let ctx = Ctx {
        caller: ctx.caller,
        project: ctx.project,
    };
    with::<E, _>(|ext| crate::dispatch(ext, &mut Vorn, &method, parsed(&args), &ctx))
        .map(|v| v.to_string())
        .map_err(to_wit)
}

pub fn on_event<E: Extension>(batch: Vec<types::Event>) {
    let batch: Vec<Event> = batch
        .into_iter()
        .map(|e| Event {
            name: e.name,
            data: parsed(&e.data),
        })
        .collect();
    let _ = with::<E, _>(|ext| {
        ext.on_events(&mut Vorn, batch);
        Ok(())
    });
}

//! A host that starts extension services and nothing else: built with and
//! without the `wasm` feature, the difference in size is what the tier adds to vornd.

use std::sync::Arc;

use serde_json::{json, Value};
use vorn_extensions::api::types::{CallCtx, Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::Bus;
use vorn_extensions::gate::{Answer, Caller, World};
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

struct Nothing;

impl World for Nothing {
    fn call(&self, _: &Caller, _: &str, _: Value) -> Answer<Value> {
        Box::pin(async { Ok(Value::Null) })
    }

    fn fetch(&self, _: &Caller, _: Request) -> Answer<Response> {
        Box::pin(async { Err(ExtError::new("unsupported", "no network here")) })
    }

    fn secret(&self, _: &Caller, _: &str, _: &str) -> Answer<Option<String>> {
        Box::pin(async { Ok(None) })
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(home), Some(id)) = (args.next(), args.next()) else {
        eprintln!("usage: ext-tiers-probe <extensions dir> <id>");
        std::process::exit(2);
    };
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let answer = rt.block_on(async {
        let router = Router::new(
            PackStore::new(std::path::PathBuf::from(home)),
            Lifecycle::new(Policy::default()),
            Arc::new(Nothing),
            RouterSettings::new(Vec::new(), "probe"),
            Bus::new(),
        );
        let ctx = CallCtx {
            caller: "probe".into(),
            project: None,
        };
        router.call(&id, "echo", json!({}), ctx).await
    });
    println!("{answer:?}");
}

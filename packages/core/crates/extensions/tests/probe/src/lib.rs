//! A component for the Wasm tier's tests: each action exercises one import
//! of the extension world, or one way a guest can misbehave.

use vorn_extension_sdk::tree::sdk::*;
use vorn_extension_sdk::tree::{self, Surface};
use vorn_extension_sdk::{
    arg, export, json, Ctx, Error, Event, Extension, Host, Init, Op, SessionStart, Statement, Value,
};

struct Probe {
    init: Init,
    events: u64,
}

fn text(args: &Value, key: &str) -> String {
    args[key].as_str().unwrap_or_default().to_owned()
}

impl Extension for Probe {
    fn init(init: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Probe {
            init: init.clone(),
            events: 0,
        })
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        ctx: &Ctx,
    ) -> Result<Value, Error> {
        match method {
            "echo" => Ok(args),
            "whoami" => Ok(json!({
                "id": self.init.id, "version": self.init.version, "vorn": self.init.vorn,
                "caller": ctx.caller, "events": self.events,
            })),
            "spin" => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "grow" => {
                let mb = args["mb"].as_u64().unwrap_or(1);
                let held = vec![1u8; usize::try_from(mb << 20).unwrap_or(usize::MAX)];
                Ok(json!(held.iter().map(|b| u64::from(*b)).sum::<u64>()))
            }
            "crash" => panic!("the probe was asked to crash"),
            "sleep" => {
                std::thread::sleep(std::time::Duration::from_millis(
                    args["ms"].as_u64().unwrap_or(10),
                ));
                Ok(Value::Null)
            }
            "call" => host.call(arg(&args, "method", method)?, args["args"].clone()),
            "publish" => {
                let ops = args["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| Op::Upsert(r.clone()))
                    .collect();
                host.publish(arg(&args, "collection", method)?, ops);
                Ok(Value::Null)
            }
            "rows" => host
                .rows(arg(&args, "collection", method)?)
                .map(Value::from),
            "start" => {
                let req = SessionStart {
                    project: text(&args, "project"),
                    agent: args["agent"].as_str().map(str::to_owned),
                    prompt: args["prompt"].as_str().map(str::to_owned),
                    headless: args["headless"].as_bool().unwrap_or(false),
                    ..SessionStart::default()
                };
                host.start_session(&req).map(Value::from)
            }
            "put" => host
                .put_file(&text(&args, "name"), text(&args, "text").as_bytes())
                .map(|()| Value::Null),
            "get" => Ok(host.file(&text(&args, "name"))?.map_or(Value::Null, |b| {
                Value::from(String::from_utf8_lossy(&b).into_owned())
            })),
            "list" => Ok(json!(host
                .files(&text(&args, "prefix"))?
                .into_iter()
                .map(|b| json!({ "name": b.name, "size": b.size }))
                .collect::<Vec<_>>())),
            "delete" => host.delete_file(&text(&args, "name")).map(Value::from),
            "tx" => {
                let statements: Vec<Statement> = args["statements"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| {
                        Statement::new(
                            s["sql"].as_str().unwrap_or_default(),
                            s["params"].as_array().cloned().unwrap_or_default(),
                        )
                    })
                    .collect();
                host.transaction(&statements).map(Value::from)
            }
            "prefix" => host.prefix().map(Value::from),
            "set" => host
                .set(&text(&args, "key"), &args["value"])
                .map(|()| Value::Null),
            "kv" => Ok(host.get(&text(&args, "key"))?.unwrap_or(Value::Null)),
            "fs" => Ok(json!({
                "root": std::fs::read_dir("/").map(|_| "read").unwrap_or("refused"),
                "write": std::fs::write("probe.txt", b"x").map(|()| "wrote").unwrap_or("refused"),
                "env": std::env::vars().count(),
                "tcp": std::net::TcpStream::connect("127.0.0.1:9").map(|_| "connected").unwrap_or("refused"),
            })),
            other => Err(Error::unknown(other)),
        }
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: Vec<Event>) {
        self.events += u64::try_from(batch.len()).unwrap_or(0);
        let ops = batch
            .into_iter()
            .map(|e| Op::Upsert(json!({ "id": e.name, "data": e.data })))
            .collect();
        host.publish("events", ops);
    }

    fn render(
        &mut self,
        host: &mut dyn Host,
        point: &str,
        s: Surface,
    ) -> Result<tree::Node, Error> {
        let n = host.rows("events")?.len();
        let on = if s.cells { "cells" } else { "pixels" };
        Ok(col([
            text_node(format!("{point} on {on}")),
            stat(n.to_string(), "Events"),
            button("hello", "Hello").icon("hand"),
        ]))
    }

    fn event(&mut self, host: &mut dyn Host, _: &str, e: tree::Event) -> Result<Value, Error> {
        host.publish("clicked", vec![Op::Upsert(json!({ "id": e.id }))]);
        Ok(json!({ "toast": format!("{} clicked", e.id) }))
    }
}

fn text_node(s: String) -> tree::Node {
    vorn_extension_sdk::tree::sdk::text(s)
}

export!(Probe);

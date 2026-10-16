//! `vorn events`: the server's public events, one line each, until stopped.

use serde_json::{json, Value};

use crate::client::Context;
use crate::exit::ExitCode;
use crate::js::{self, field};
use crate::output::short_id;

pub const EVENTS_USAGE: &str = "Usage
  vorn events [--json] [--filter <event,...>] [--session <target>]

Streams what the server tells extensions, as it happens, until you stop it.
With --json each line is one object, {\"v\":1,\"event\":...,\"at\":<ms>,
\"session\":...,\"data\":{...}}; \"session\" is present for events about a session.

Events
  session.created   session.status   session.exited   session.mark
  session.program   agent.hook       git.changed
  task.created      task.statusChanged

Options
  --filter <names>    Only these events, comma separated
  --session <target>  Only events about one session (a target as for `session read`)
";

/// `vorn events`.
pub async fn run(ctx: &mut Context<'_>) -> ExitCode {
    if ctx.args.help {
        ctx.io.write(EVENTS_USAGE);
        return ExitCode::Ok;
    }
    if ctx.args.positionals.len() > 1 {
        return ctx.usage("events takes no operands", EVENTS_USAGE);
    }
    if !ctx.server().await {
        return ExitCode::Unreachable;
    }
    let mut params = serde_json::Map::new();
    if !ctx.args.filter.is_empty() {
        params.insert("events".into(), json!(ctx.args.filter));
    }
    if let Some(given) = ctx.args.session.clone() {
        match crate::session::resolve_session(ctx, &given).await {
            Ok(found) => params.insert("session".into(), found.id.into()),
            Err(err) => return ctx.error("could not find the session", err),
        };
    }
    let mut events = match ctx.rpc.events(Value::Object(params)).await {
        Ok(events) => events,
        Err(err) => return ctx.failed("could not follow events", err),
    };
    loop {
        match events.next().await {
            Ok(event) => {
                let line = if ctx.args.json {
                    event.to_string()
                } else {
                    let session = field(&event, "session")
                        .and_then(Value::as_str)
                        .map_or(String::new(), |s| format!(" {}", short_id(s)));
                    format!(
                        "{} {}{session} {}",
                        js::string(field(&event, "at")),
                        js::string(field(&event, "event")),
                        field(&event, "data").unwrap_or(&Value::Null)
                    )
                };
                ctx.io.write(&format!("{line}\n"));
            }
            Err(err) => return ctx.failed("the event stream ended", err),
        }
    }
}

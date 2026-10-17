//! Agent footer: keeps one row per agent session from the events Vorn sends
//! (the model it started with, how many tools its hooks reported, the last
//! one) and draws it under the session's card as an element tree.

use vorn_extension_sdk::tree::sdk::*;
use vorn_extension_sdk::tree::{Node, Surface, TextStyle};
use vorn_extension_sdk::{json, Ctx, Error, Event, Extension, Host, Init, Op, Value};

const SESSIONS: &str = "sessions";

/// The extension; its rows hold what it knows.
#[derive(Debug, Default)]
pub struct Footer;

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

/// What one batch of events changes in the rows.
fn changes(rows: &[Value], batch: &[Event]) -> Vec<Op> {
    let mut rows = rows.to_vec();
    let mut ops = Vec::new();
    for e in batch {
        let id = s(&e.data, "sessionId").to_owned();
        match e.name.as_str() {
            "session.created" if !matches!(s(&e.data, "agent"), "" | "shell") => {
                let row = json!({ "id": id, "model": s(&e.data, "model"), "tool": "", "tools": 0 });
                rows.retain(|r| s(r, "id") != id);
                rows.push(row.clone());
                ops.push(Op::Upsert(row));
            }
            "session.exited" if rows.iter().any(|r| s(r, "id") == id) => {
                rows.retain(|r| s(r, "id") != id);
                ops.push(Op::Delete(id));
            }
            "agent.hook" if s(&e.data, "event") == "PreToolUse" => {
                let Some(row) = rows.iter_mut().find(|r| s(r, "id") == id) else {
                    continue;
                };
                row["tools"] = json!(row["tools"].as_u64().unwrap_or(0) + 1);
                if let Some(tool) = e.data["tool"].as_str().filter(|t| !t.is_empty()) {
                    row["tool"] = json!(tool);
                }
                ops.push(Op::Upsert(row.clone()));
            }
            _ => {}
        }
    }
    ops
}

/// Each agent's footer, once its first tool call is reported.
fn footers(rows: &[Value]) -> Node {
    per_session(rows.iter().filter(|r| r["tools"].as_u64() > Some(0)).map(|r| {
        let said = format!("{} tool calls · last {}", r["tools"], s(r, "tool"));
        let footer = row([
            icon("wrench"),
            text(said).style(TextStyle::Secondary).grow(),
            text(s(r, "model")).style(TextStyle::Secondary),
        ])
        .gap(8.0);
        (s(r, "id"), footer)
    }))
}

impl Extension for Footer {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Footer, Error> {
        Ok(Footer)
    }

    /// It has no actions; a call only starts it, so it hears the next session start.
    fn handle(&mut self, _: &mut dyn Host, _: &str, _: Value, _: &Ctx) -> Result<Value, Error> {
        Ok(Value::Null)
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: Vec<Event>) {
        let rows = host.rows(SESSIONS).unwrap_or_default();
        let ops = changes(&rows, &batch);
        if !ops.is_empty() {
            host.publish(SESSIONS, ops);
        }
    }

    fn render(&mut self, host: &mut dyn Host, _: &str, _: Surface) -> Result<Node, Error> {
        Ok(footers(&host.rows(SESSIONS)?))
    }
}

vorn_extension_sdk::export!(Footer);

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_extension_sdk::testing::FakeHost;
    use vorn_extension_sdk::tree;

    fn event(name: &str, data: Value) -> Event {
        Event {
            name: name.into(),
            data,
        }
    }

    #[test]
    fn an_agents_tool_calls_draw_under_its_card_once_there_is_one() {
        let mut h = FakeHost::default();
        let mut f = Footer::init(&Init::default(), &mut h).unwrap();
        f.on_events(
            &mut h,
            vec![
                event("session.created", json!({ "sessionId": "a", "agent": "acme", "model": "mini" })),
                event("session.created", json!({ "sessionId": "s", "agent": "shell" })),
            ],
        );
        assert_eq!(h.rows(SESSIONS).unwrap().len(), 1, "a shell has no footer");
        let drawn = f.render(&mut h, "tools", Surface::default()).unwrap();
        assert!(tree::for_session(&drawn, "a").is_none(), "no tool call yet");
        let hook = |tool: &str| event("agent.hook", json!({ "sessionId": "a", "event": "PreToolUse", "tool": tool }));
        f.on_events(&mut h, vec![hook("read"), hook("edit")]);
        let drawn = f.render(&mut h, "tools", Surface::default()).unwrap();
        tree::check(&drawn).unwrap();
        let text = vorn_extension_sdk::serde_json::to_string(tree::for_session(&drawn, "a").unwrap()).unwrap();
        assert!(text.contains("2 tool calls · last edit"), "{text}");
        assert!(text.contains("mini"), "{text}");
        f.on_events(&mut h, vec![event("session.exited", json!({ "sessionId": "a" }))]);
        assert!(h.rows(SESSIONS).unwrap().is_empty());
    }
}

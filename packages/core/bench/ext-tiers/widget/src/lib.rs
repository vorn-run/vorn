//! The Widget: a small window over the others listing every agent's status
//! and the tools they wait to use, with Allow and Deny, the rules the agent
//! offers to keep, and the questions it asks; it folds to a line of counts.
//!
//! Vorn ships it as a Wasm component (`vorn-extension.toml` beside this
//! crate). What it shows lives in the rows it publishes, so a restart of its
//! instance loses nothing a person sees, and its windows are element trees
//! drawn from those rows.

use std::collections::BTreeSet;

use vorn_extension_sdk::tree::{self, Surface};
use vorn_extension_sdk::{arg, json, Ctx, Error, Event, Extension, Host, Init, Op, Value};

mod view;

const NAME: &str = "Widget";
const AGENTS: &str = "agents";
const PERMISSIONS: &str = "permissions";
const SUGGESTIONS: &str = "suggestions";
const OPTIONS: &str = "options";
const SESSION_CREATED: &str = "session.created";
const SESSION_STATUS: &str = "session.status";
const SESSION_EXITED: &str = "session.exited";
const AGENT_PERMISSION: &str = "agent.permission";
const AGENT_PERMISSION_CLOSED: &str = "agent.permissionClosed";
/// The longest rule shown on a suggestion's button, in characters.
const RULE_MAX: usize = 30;
/// The longest tool input shown, in characters.
const INPUT_MAX: usize = 160;

/// The extension; everything it knows is in its rows.
#[derive(Debug, Default)]
pub struct Widget;

/// Every running session, read again: a status change carries no name.
fn agents(host: &mut dyn Host) {
    let Ok(Value::Array(sessions)) = host.call("sessions.list", json!({})) else {
        return;
    };
    let mut ops = vec![Op::Clear];
    ops.extend(sessions.iter().map(|s| {
        Op::Upsert(json!({
            "id": s["id"],
            "name": s["name"],
            "agent": s["agent"],
            "status": s["status"],
        }))
    }));
    host.publish(AGENTS, ops);
}

/// Text `s` cut to `max` characters, an ellipsis ending a cut one.
fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let kept: String = s.chars().take(max - 1).collect();
    format!("{kept}…")
}

/// What a request asks, in a line: its description, else its command or file.
fn asked(data: &Value) -> String {
    let said = data["description"].as_str().unwrap_or_default().trim();
    if !said.is_empty() {
        return said.to_owned();
    }
    let input = &data["toolInput"];
    let line = ["command", "file_path", "path", "url", "pattern"]
        .iter()
        .find_map(|k| input[*k].as_str())
        .map_or_else(|| input.to_string(), str::to_owned);
    cut(&line, INPUT_MAX)
}

/// What the button of a rule the agent offers to keep says.
fn suggestion_label(sg: &Value, tool: &str) -> String {
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    match sg["type"].as_str() {
        Some("addRules") => {
            let first = &sg["rules"][0];
            let name = first["toolName"].as_str().unwrap_or(tool);
            let rule = text(first, "ruleContent");
            if rule.contains("**") {
                let trimmed = rule.replace("/**", "");
                let dir = trimmed.rsplit('/').find(|d| !d.is_empty()).unwrap_or(&rule);
                format!("Allow {name} in {dir}/")
            } else if !rule.is_empty() {
                format!("Always allow `{}`", cut(&rule, RULE_MAX))
            } else {
                format!("Always allow {name}")
            }
        }
        Some("setMode") => match sg["mode"].as_str() {
            Some("acceptEdits") => "Auto-accept edits".to_owned(),
            Some("plan") => "Switch to plan mode".to_owned(),
            Some(m) => m.to_owned(),
            None => "Set mode".to_owned(),
        },
        other => sg["label"]
            .as_str()
            .or(other)
            .unwrap_or_default()
            .to_owned(),
    }
}

/// An agent asks to use a tool: the request, the rules it offers and the options of its questions.
fn permission(host: &mut dyn Host, data: &Value) {
    let Some(id) = data["requestId"].as_str() else {
        return;
    };
    let terminal = data["terminalId"].as_str().unwrap_or_default();
    let agent = host
        .rows(AGENTS)
        .unwrap_or_default()
        .into_iter()
        .find(|a| a["id"] == terminal)
        .and_then(|a| a["name"].as_str().map(str::to_owned))
        .or_else(|| data["projectName"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let tool = data["toolName"].as_str().unwrap_or("Tool");
    let list = |k: &str| data[k].as_array().cloned().unwrap_or_default();
    let questions = list("questions");
    let suggestions = list("permissionSuggestions")
        .iter()
        .enumerate()
        .map(|(i, sg)| {
            Op::Upsert(json!({
                "id": format!("{id}:{i}"),
                "request": id,
                "label": suggestion_label(sg, tool),
            }))
        })
        .collect();
    host.publish(SUGGESTIONS, suggestions);
    let mut options = Vec::new();
    for (q, question) in questions.iter().enumerate() {
        let header = question["header"].as_str().unwrap_or_default();
        let said = question["question"].as_str().unwrap_or_default();
        let ask = if header.is_empty() {
            said.to_owned()
        } else {
            format!("{}: {said}", header.to_uppercase())
        };
        let labels = question["options"].as_array().into_iter().flatten();
        for (o, option) in labels.enumerate() {
            options.push(Op::Upsert(json!({
                "id": format!("{id}:{q}:{o}"),
                "request": id,
                "ask": if o == 0 { ask.as_str() } else { "" },
                "label": option["label"].as_str().unwrap_or_default(),
                "picked": false,
            })));
        }
    }
    host.publish(OPTIONS, options);
    let row = json!({
        "id": id,
        "tool": tool,
        "agent": agent,
        "description": asked(data),
        "question": !questions.is_empty(),
        "ready": false,
    });
    host.publish(PERMISSIONS, vec![Op::Upsert(row)]);
}

/// Forgets request `id` and its suggestions and options.
fn forget(host: &mut dyn Host, id: &str) {
    host.publish(PERMISSIONS, vec![Op::Delete(id.to_owned())]);
    for collection in [SUGGESTIONS, OPTIONS] {
        let gone = host
            .rows(collection)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r["request"] == id)
            .filter_map(|r| r["id"].as_str().map(|i| Op::Delete(i.to_owned())))
            .collect();
        host.publish(collection, gone);
    }
}

/// Allows request `id` with `answer`'s fields added, and forgets it.
fn resolve(host: &mut dyn Host, id: &str, answer: Value) -> Result<Value, Error> {
    let mut params = json!({ "requestId": id, "allow": true });
    if let (Some(p), Value::Object(a)) = (params.as_object_mut(), answer) {
        p.extend(a);
    }
    host.call("permissions.resolve", params)?;
    forget(host, id);
    Ok(Value::Null)
}

/// `allow` or `deny`: answers the request as it is.
fn answer(host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
    let id = arg(args, "id", action)?;
    let allow = action == "allow";
    host.call(
        "permissions.resolve",
        json!({ "requestId": id, "allow": allow }),
    )?;
    forget(host, id);
    Ok(Value::Null)
}

/// `suggest`: Allow, keeping the rule the pressed suggestion offers.
fn suggest(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
    let row = arg(args, "id", "suggest")?;
    let (id, index) = row
        .rsplit_once(':')
        .and_then(|(id, i)| Some((id, i.parse::<u64>().ok()?)))
        .ok_or_else(|| Error::bad_args(format!("no suggestion {row}")))?;
    resolve(host, id, json!({ "suggestion": index }))
}

/// The question and request option row `row` answers, as `(request, "request:question:")`.
fn question_of(row: &str) -> Option<(&str, String)> {
    let (rest, _) = row.rsplit_once(':')?;
    let (request, _) = rest.rsplit_once(':')?;
    Some((request, format!("{rest}:")))
}

/// `pick`: chooses an option, in place of the question's other options.
fn pick(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
    let row = arg(args, "id", "pick")?;
    let (request, question) =
        question_of(row).ok_or_else(|| Error::bad_args(format!("no option {row}")))?;
    let mut ops = Vec::new();
    let mut options = host.rows(OPTIONS)?;
    for o in &mut options {
        let Some(oid) = o["id"].as_str() else {
            continue;
        };
        if oid.starts_with(&question) {
            let picked = oid == row;
            if o["picked"].as_bool() != Some(picked) {
                o["picked"] = json!(picked);
                ops.push(Op::Upsert(o.clone()));
            }
        }
    }
    host.publish(OPTIONS, ops);
    let ready = answers(&options, request).is_some();
    if let Some(mut p) = host
        .rows(PERMISSIONS)?
        .into_iter()
        .find(|p| p["id"] == request)
    {
        p["ready"] = json!(ready);
        host.publish(PERMISSIONS, vec![Op::Upsert(p)]);
    }
    Ok(Value::Null)
}

/// Request `request`'s answers, keyed by question index, once every question has one.
fn answers(options: &[Value], request: &str) -> Option<Value> {
    let mut asked = BTreeSet::new();
    let mut answers = vorn_extension_sdk::serde_json::Map::new();
    for o in options.iter().filter(|o| o["request"] == request) {
        let (_, question) = question_of(o["id"].as_str()?)?;
        let index = question
            .trim_end_matches(':')
            .rsplit(':')
            .next()?
            .to_owned();
        asked.insert(index.clone());
        if o["picked"].as_bool() == Some(true) {
            answers.insert(index, o["label"].clone());
        }
    }
    (!asked.is_empty() && asked.len() == answers.len()).then_some(Value::Object(answers))
}

/// `submit`: Allow with the options picked as the answers to the agent's questions.
fn submit(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
    let id = arg(args, "id", "submit")?;
    let answers = answers(&host.rows(OPTIONS)?, id)
        .ok_or_else(|| Error::bad_args("every question needs an answer"))?;
    resolve(host, id, json!({ "answers": answers }))
}

fn act(host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
    match action {
        "allow" | "deny" => answer(host, action, args),
        "suggest" => suggest(host, args),
        "pick" => pick(host, args),
        "submit" => submit(host, args),
        other => Err(Error::new(
            "unknown-action",
            format!("{NAME} has no action {other}"),
        )),
    }
}

impl Extension for Widget {
    /// A fresh instance reads the sessions again, as switching it on did.
    fn init(_: &Init, host: &mut dyn Host) -> Result<Widget, Error> {
        agents(host);
        Ok(Widget)
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        act(host, method, &args)
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: Vec<Event>) {
        let mut sessions = false;
        for event in &batch {
            match event.name.as_str() {
                AGENT_PERMISSION => permission(host, &event.data),
                AGENT_PERMISSION_CLOSED => {
                    if let Some(id) = event.data["requestId"].as_str() {
                        forget(host, id);
                    }
                }
                SESSION_CREATED | SESSION_STATUS | SESSION_EXITED => sessions = true,
                _ => {}
            }
        }
        if sessions {
            agents(host);
        }
    }

    fn render(
        &mut self,
        host: &mut dyn Host,
        point: &str,
        _: Surface,
    ) -> Result<tree::Node, Error> {
        let agents = host.rows(AGENTS)?;
        match point {
            "compact" => Ok(view::compact(&agents)),
            "widget" => {
                let asks = view::Asks {
                    permissions: host.rows(PERMISSIONS)?,
                    suggestions: host.rows(SUGGESTIONS)?,
                    options: host.rows(OPTIONS)?,
                };
                Ok(view::widget(&agents, &asks))
            }
            other => Err(Error::new(
                "unknown-action",
                format!("{NAME} draws no {other}"),
            )),
        }
    }

    fn event(&mut self, host: &mut dyn Host, _: &str, e: tree::Event) -> Result<Value, Error> {
        let (does, row) = e.id.split_once(':').unwrap_or((e.id.as_str(), ""));
        match does {
            "collapse" => Ok(json!({ "window": "compact" })),
            "expand" => Ok(json!({ "window": "widget" })),
            "vorn" => Ok(json!({ "show": true })),
            "show" => Ok(json!({ "focus": row })),
            "allow" | "deny" | "suggest" | "pick" | "submit" if !row.is_empty() => {
                act(host, does, &json!({ "id": row }))
            }
            _ => Err(Error::bad_args(format!("no button is '{}'", e.id))),
        }
    }
}

vorn_extension_sdk::export!(Widget);

#[cfg(test)]
mod tests;

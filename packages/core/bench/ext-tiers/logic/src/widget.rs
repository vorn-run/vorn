//! The Widget, ported from vornd's built-in: a small window over the others
//! listing every agent's status and the tools they wait to use, with Allow
//! and Deny, the rules the agent offers to keep, and the questions it asks;
//! it folds to a line of counts.

use serde_json::{json, Value};

use crate::{arg, unknown, Error, Extension, Host, Op, Rows};

const NAME: &str = "Widget";
const AGENTS: &str = "agents";
const PERMISSIONS: &str = "permissions";
const SUGGESTIONS: &str = "suggestions";
const OPTIONS: &str = "options";
/// The longest rule shown on a suggestion's button, in characters.
const RULE_MAX: usize = 30;
/// The longest tool input shown, in characters.
const INPUT_MAX: usize = 160;

pub const SESSION_CREATED: &str = "session.created";
pub const SESSION_STATUS: &str = "session.status";
pub const SESSION_EXITED: &str = "session.exited";
pub const AGENT_PERMISSION: &str = "agent.permission";
pub const AGENT_PERMISSION_CLOSED: &str = "agent.permissionClosed";

#[derive(Default)]
pub struct Widget {
    rows: Rows,
}

/// What a request asks, in a line: its description, else its command or file.
pub fn asked(data: &Value) -> String {
    let said = data["description"].as_str().unwrap_or_default().trim();
    if !said.is_empty() {
        return said.to_owned();
    }
    let input = &data["toolInput"];
    let line = ["command", "file_path", "path", "url", "pattern"]
        .iter()
        .find_map(|k| input[*k].as_str())
        .map_or_else(|| input.to_string(), str::to_owned);
    if line.chars().count() > INPUT_MAX {
        let cut: String = line.chars().take(INPUT_MAX - 1).collect();
        return format!("{cut}…");
    }
    line
}

/// Text `s` cut to `max` characters, an ellipsis ending a cut one.
fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let kept: String = s.chars().take(max - 1).collect();
    format!("{kept}…")
}

/// What the button of a rule the agent offers to keep says.
pub fn suggestion_label(sg: &Value, tool: &str) -> String {
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

/// The question and request option row `row` answers, as `(request, "request:question:")`.
fn question_of(row: &str) -> Option<(&str, String)> {
    let (rest, _) = row.rsplit_once(':')?;
    let (request, _) = rest.rsplit_once(':')?;
    Some((request, format!("{rest}:")))
}

/// Request `request`'s answers, keyed by question index, once every question has one.
pub fn answers(options: &[Value], request: &str) -> Option<Value> {
    let mut asked = std::collections::BTreeSet::new();
    let mut answers = serde_json::Map::new();
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
    (!asked.is_empty() && asked.len() == answers.len()).then(|| Value::Object(answers))
}

impl Widget {
    /// Every running session, read again: a status change carries no name.
    fn agents(&mut self, host: &mut dyn Host) {
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
        self.rows.publish(host, AGENTS, ops);
    }

    fn permission(&mut self, host: &mut dyn Host, data: &Value) {
        let Some(id) = data["requestId"].as_str() else {
            return;
        };
        let terminal = data["terminalId"].as_str().unwrap_or_default();
        let agent = self
            .rows
            .rows(AGENTS)
            .into_iter()
            .find(|a| a["id"] == terminal)
            .and_then(|a| a["name"].as_str().map(str::to_owned))
            .or_else(|| data["projectName"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let tool = data["toolName"].as_str().unwrap_or("Tool");
        let list = |k: &str| data[k].as_array().cloned().unwrap_or_default();
        let questions = list("questions");
        let suggestions = list("permissionSuggestions");
        let suggestions = suggestions.iter().enumerate().map(|(i, sg)| {
            Op::Upsert(json!({
                "id": format!("{id}:{i}"),
                "request": id,
                "label": suggestion_label(sg, tool),
            }))
        });
        self.rows.publish(host, SUGGESTIONS, suggestions.collect());
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
        self.rows.publish(host, OPTIONS, options);
        let row = json!({
            "id": id,
            "tool": tool,
            "agent": agent,
            "description": asked(data),
            "question": !questions.is_empty(),
            "ready": false,
        });
        self.rows.publish(host, PERMISSIONS, vec![Op::Upsert(row)]);
    }

    /// Forgets request `id` and its suggestions and options.
    fn forget(&mut self, host: &mut dyn Host, id: &str) {
        self.rows
            .publish(host, PERMISSIONS, vec![Op::Delete(id.to_owned())]);
        for collection in [SUGGESTIONS, OPTIONS] {
            let gone = (self.rows.rows(collection).into_iter())
                .filter(|r| r["request"] == id)
                .filter_map(|r| r["id"].as_str().map(|i| Op::Delete(i.to_owned())))
                .collect();
            self.rows.publish(host, collection, gone);
        }
    }

    fn resolve(&mut self, host: &mut dyn Host, id: &str, answer: Value) -> Result<Value, Error> {
        let mut params = json!({ "requestId": id, "allow": true });
        if let (Some(p), Some(a)) = (params.as_object_mut(), answer.as_object()) {
            p.extend(a.clone());
        }
        host.call("permissions.resolve", params)?;
        self.forget(host, id);
        Ok(Value::Null)
    }

    /// `suggest`: Allow, keeping the rule the pressed suggestion offers.
    fn suggest(&mut self, host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
        let row = arg(args, "id", "suggest")?;
        let (id, index) = row
            .rsplit_once(':')
            .and_then(|(id, i)| Some((id, i.parse::<u64>().ok()?)))
            .ok_or_else(|| Error::new("bad-request", format!("no suggestion {row}")))?;
        self.resolve(host, id, json!({ "suggestion": index }))
    }

    /// `pick`: chooses an option, in place of the question's other options.
    fn pick(&mut self, host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
        let row = arg(args, "id", "pick")?;
        let (request, question) = question_of(row)
            .ok_or_else(|| Error::new("bad-request", format!("no option {row}")))?;
        let mut ops = Vec::new();
        let mut options = self.rows.rows(OPTIONS);
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
        self.rows.publish(host, OPTIONS, ops);
        let ready = answers(&options, request).is_some();
        if let Some(mut p) = self
            .rows
            .rows(PERMISSIONS)
            .into_iter()
            .find(|p| p["id"] == request)
        {
            p["ready"] = json!(ready);
            self.rows.publish(host, PERMISSIONS, vec![Op::Upsert(p)]);
        }
        Ok(Value::Null)
    }

    /// `submit`: Allow with the options picked as the answers to the agent's questions.
    fn submit(&mut self, host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
        let id = arg(args, "id", "submit")?;
        let answers = answers(&self.rows.rows(OPTIONS), id)
            .ok_or_else(|| Error::new("bad-request", "every question needs an answer"))?;
        self.resolve(host, id, json!({ "answers": answers }))
    }

    fn answer(&mut self, host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        let id = arg(args, "id", action)?;
        let allow = action == "allow";
        host.call(
            "permissions.resolve",
            json!({ "requestId": id, "allow": allow }),
        )?;
        self.forget(host, id);
        Ok(Value::Null)
    }
}

impl Extension for Widget {
    fn init(&mut self, host: &mut dyn Host) {
        self.agents(host);
    }

    fn handle(&mut self, host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        match action {
            "allow" | "deny" => self.answer(host, action, args),
            "suggest" => self.suggest(host, args),
            "pick" => self.pick(host, args),
            "submit" => self.submit(host, args),
            other => Err(unknown(NAME, other)),
        }
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: &[(String, Value)]) {
        let mut sessions = false;
        for (name, data) in batch {
            match name.as_str() {
                AGENT_PERMISSION => self.permission(host, data),
                AGENT_PERMISSION_CLOSED => {
                    if let Some(id) = data["requestId"].as_str() {
                        self.forget(host, id);
                    }
                }
                _ => sessions = true,
            }
        }
        if sessions {
            self.agents(host);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{answers, asked, suggestion_label};

    #[test]
    fn a_request_reads_as_its_description_or_its_input() {
        assert_eq!(
            asked(&json!({ "description": " Run the tests " })),
            "Run the tests"
        );
        assert_eq!(
            asked(&json!({ "toolInput": { "command": "cargo test" } })),
            "cargo test"
        );
        let long = "x".repeat(400);
        assert_eq!(
            asked(&json!({ "toolInput": { "command": long } }))
                .chars()
                .count(),
            160
        );
    }

    #[test]
    fn a_suggestion_reads_as_the_rule_it_keeps() {
        let rule = |r: &str| json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": r }] });
        assert_eq!(
            suggestion_label(&rule("src/**"), "Bash"),
            "Allow Bash in src/"
        );
        assert_eq!(
            suggestion_label(&rule("npm test"), "Bash"),
            "Always allow `npm test`"
        );
        assert_eq!(suggestion_label(&rule(""), "Bash"), "Always allow Bash");
        let mode = json!({ "type": "setMode", "mode": "acceptEdits" });
        assert_eq!(suggestion_label(&mode, "Edit"), "Auto-accept edits");
    }

    #[test]
    fn answers_wait_for_every_question() {
        let o = |id: &str, picked: bool| json!({ "id": id, "request": "r:1", "label": id, "picked": picked });
        let mut opts = vec![o("r:1:0:0", true), o("r:1:0:1", false), o("r:1:1:0", false)];
        assert_eq!(answers(&opts, "r:1"), None);
        opts[2]["picked"] = json!(true);
        assert_eq!(
            answers(&opts, "r:1"),
            Some(json!({ "0": "r:1:0:0", "1": "r:1:1:0" }))
        );
    }
}

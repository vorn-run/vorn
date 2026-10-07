//! Typed output of a headless agent step (`structured-output.ts`).
//!
//! An agent that declares an output schema is asked to end its run with a
//! JSON object between two marker lines. The object is pulled back out of
//! its logs, scalars the schema types are coerced, and the schema's
//! `required` keys are checked, so a branch never acts on garbage.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::js;

/// The markers the agent wraps its answer in.
pub const BEGIN: &str = "<<<VORN_OUTPUT>>>";
pub const END: &str = "<<<END_VORN_OUTPUT>>>";

static FENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)```(?:json)?\s*(.*?)```").expect("the fence pattern is valid")
});

/// The instructions appended to a prompt whose step declares a schema.
pub fn instructions(schema: &Value) -> String {
    [
        "## Required Output",
        "",
        "When you have finished, output a single JSON object that conforms to the JSON Schema below. Wrap it exactly between the two marker lines — nothing else after the closing marker:",
        "",
        BEGIN,
        "{ ...your JSON here... }",
        END,
        "",
        "Schema:",
        "",
        "```json",
        &js::stringify_pretty(schema),
        "```",
        "",
    ]
    .join("\n")
}

/// The JSON text the agent produced: the marked block (the last one), then
/// the last fenced block, then the last balanced object anywhere.
fn json_text(text: &str) -> Option<&str> {
    if let Some(begin) = text.rfind(BEGIN) {
        let after = begin + BEGIN.len();
        let block = match text[after..].find(END) {
            Some(end) => &text[after..after + end],
            None => &text[after..],
        };
        if let Some(found) = last_balanced_object(block) {
            return Some(found);
        }
    }
    let fences: Vec<&str> = FENCE
        .captures_iter(text)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect();
    if let Some(found) = fences.iter().rev().find_map(|f| last_balanced_object(f)) {
        return Some(found);
    }
    last_balanced_object(text)
}

/// The last complete top-level `{...}`, braces inside strings ignored.
fn last_balanced_object(text: &str) -> Option<&str> {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    let mut start = None;
    let mut last = None;
    for (i, b) in text.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start.take() {
                        last = Some(&text[s..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    last
}

/// A property's `type`, the first non-null one of a union.
fn type_hint(prop: &Value) -> Option<&str> {
    match prop.get("type")? {
        Value::String(t) => Some(t),
        Value::Array(types) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .or_else(|| types.first().and_then(Value::as_str)),
        _ => None,
    }
}

/// Strings at the top level made the scalar their property declares.
fn coerce(mut obj: Map<String, Value>, schema: &Value) -> Map<String, Value> {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return obj;
    };
    for (key, prop) in props {
        let Some(Value::String(raw)) = obj.get(key) else {
            continue;
        };
        let coerced = match type_hint(prop) {
            Some("number" | "integer") if !raw.trim().is_empty() => js::number(raw)
                .filter(|n| !n.is_nan())
                .map(js::number_value),
            Some("boolean") if raw == "true" => Some(Value::Bool(true)),
            Some("boolean") if raw == "false" => Some(Value::Bool(false)),
            _ => None,
        };
        if let Some(v) = coerced.filter(|v| !v.is_null()) {
            obj.insert(key.clone(), v);
        }
    }
    obj
}

/// `extractStructuredOutput`: the object the agent produced, or why there
/// is none.
pub fn extract(logs: &str, schema: &Value) -> Result<Map<String, Value>, String> {
    let text = json_text(logs).ok_or("No JSON output block found in the agent output.")?;
    let parsed: Map<String, Value> = serde_json::from_str(text)
        .map_err(|_| "The agent output was not valid JSON.".to_owned())?;
    let coerced = coerce(parsed, schema);
    let missing: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|req| {
            req.iter()
                .filter_map(Value::as_str)
                .filter(|k| !coerced.contains_key(*k))
                .collect()
        })
        .unwrap_or_default();
    if !missing.is_empty() {
        return Err(format!(
            "Agent output is missing required field(s): {}.",
            missing.join(", ")
        ));
    }
    Ok(coerced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({ "type": "object", "properties": { "n": { "type": "integer" }, "ok": { "type": ["boolean", "null"] }, "s": { "type": "string" } }, "required": ["n"] })
    }

    #[test]
    fn prefers_the_last_marked_block() {
        let logs = format!("{BEGIN}\n{{\"n\": 1}}\n{END}\nlater {BEGIN} {{\"n\": \"2\", \"ok\": \"true\", \"s\": \"{{x}}\"}} {END} tail {{\"n\": 9}}");
        let out = extract(&logs, &schema()).unwrap();
        assert_eq!(
            Value::Object(out),
            json!({ "n": 2, "ok": true, "s": "{x}" })
        );
    }

    #[test]
    fn falls_back_to_fences_then_any_object() {
        let fenced = "text ```json\n{\"n\": 3}\n``` and {\"n\": 4} ```\nnot json\n```";
        assert_eq!(extract(fenced, &schema()).unwrap()["n"], json!(3));
        assert_eq!(
            extract("say {\"n\": 5} done", &schema()).unwrap()["n"],
            json!(5)
        );
    }

    #[test]
    fn says_why_there_is_no_output() {
        assert_eq!(
            extract("nothing", &schema()).unwrap_err(),
            "No JSON output block found in the agent output."
        );
        assert_eq!(
            extract("{bad}", &schema()).unwrap_err(),
            "The agent output was not valid JSON."
        );
        assert_eq!(
            extract("{\"s\": \"x\"}", &schema()).unwrap_err(),
            "Agent output is missing required field(s): n."
        );
        // A number that is not one stays text.
        assert_eq!(
            extract("{\"n\": \"12px\"}", &schema()).unwrap()["n"],
            json!("12px")
        );
    }

    #[test]
    fn instructions_carry_the_markers_and_schema() {
        let text = instructions(&json!({ "a": 1 }));
        assert!(text.starts_with("## Required Output\n"));
        assert!(text.contains(BEGIN) && text.contains(END));
        assert!(text.ends_with("```json\n{\n  \"a\": 1\n}\n```\n"));
    }
}

//! `{{…}}` templates, resolved against a run's context and the outputs of
//! the steps before (`template-vars.ts`).
//!
//! A token names a namespace and a dotted path: `steps`, `task`, `trigger`,
//! `connectorItem`, `inputs`, `context` and `loop`. A token whose namespace
//! this run does not have is left as written, so a misplaced one is visible
//! rather than blank; one whose namespace exists but holds nothing there
//! becomes empty text.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::js;
use crate::model::Context;

/// What the steps before produced, by slug: each an object of outputs.
pub type StepOutputs = Map<String, Value>;

/// The longest text a template expands to; longer text keeps its end.
pub const MAX_OUTPUT_LENGTH: usize = 50_000;

/// `\w` is ASCII in JavaScript.
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\{\s*([a-zA-Z_][A-Za-z0-9_.\-]*)\s*\}\}").expect("the token pattern is valid")
});

static WHOLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*\{\{\s*([a-zA-Z_][A-Za-z0-9_.\-]*)\s*\}\}\s*$")
        .expect("the whole-token pattern is valid")
});

/// What a path names.
enum Found {
    /// No namespace this run has: the token stays as written.
    Unresolved,
    /// A value; `undefined` and `null` both as [`Value::Null`].
    Value(Value),
}

/// `walkPath`: into objects by key and arrays by index (or `length`);
/// anything else, or a missing step, is undefined.
fn walk(root: &Value, path: &[&str]) -> Value {
    let mut current = root;
    let mut length;
    for segment in path {
        current = match current {
            Value::Object(map) => match map.get(*segment) {
                Some(v) => v,
                None => return Value::Null,
            },
            Value::Array(items) if *segment == "length" => {
                length = Value::from(items.len());
                &length
            }
            Value::Array(items) => match index_of(segment).and_then(|i| items.get(i)) {
                Some(v) => v,
                None => return Value::Null,
            },
            _ => return Value::Null,
        };
    }
    current.clone()
}

/// An array index as JavaScript reads a property name: digits, no leading zero.
fn index_of(segment: &str) -> Option<usize> {
    let canonical = segment == "0" || !segment.starts_with('0');
    (canonical && !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()))
        .then(|| segment.parse().ok())
        .flatten()
}

/// `stringifyResolved`: scalars as text, objects and lists as JSON, both cut
/// to their last [`MAX_OUTPUT_LENGTH`] units.
fn text_of(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => js::tail(s, MAX_OUTPUT_LENGTH).to_owned(),
        Value::Number(_) | Value::Bool(_) => js::to_string(value),
        other => js::tail(&js::stringify(other), MAX_OUTPUT_LENGTH).to_owned(),
    }
}

fn lookup(path: &str, context: Option<&Context>, outputs: Option<&StepOutputs>) -> Found {
    let mut parts = path.split('.');
    let ns = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    if rest.is_empty() {
        return Found::Unresolved;
    }
    if ns == "steps" {
        if let Some(outputs) = outputs {
            let Some(step) = outputs.get(rest[0]) else {
                return Found::Value(Value::String(String::new()));
            };
            return Found::Value(walk(step, &rest[1..]));
        }
    }
    let Some(ctx) = context else {
        return Found::Unresolved;
    };
    match ns {
        "task" => {
            if let Some(task) = &ctx.task {
                if rest.len() == 1 {
                    return Found::Value(Value::String(match task.get(rest[0]) {
                        None | Some(Value::Null) => String::new(),
                        Some(v) => js::to_string(v),
                    }));
                }
                return Found::Value(walk(task, &rest));
            }
        }
        "trigger" => {
            if let Some(trigger) = &ctx.trigger {
                return Found::Value(walk(trigger, &rest));
            }
        }
        "connectorItem" => {
            if let Some(item) = &ctx.connector_item {
                return Found::Value(walk(item, &rest));
            }
        }
        "inputs" => {
            if let Some(inputs) = &ctx.inputs {
                return Found::Value(match inputs.get(rest[0]) {
                    None => Value::String(String::new()),
                    Some(v) if rest.len() == 1 => v.clone(),
                    Some(v) => walk(v, &rest[1..]),
                });
            }
        }
        "context" if rest.len() == 1 => {
            return Found::Value(Value::String(
                context_field(rest[0], ctx).map_or_else(String::new, |f| f.text()),
            ));
        }
        "loop" => {
            if let Some(pass) = &ctx.pass {
                let key = rest[0];
                let deeper = &rest[1..];
                if key == "item" {
                    let item = pass.item.clone().unwrap_or(Value::Null);
                    return Found::Value(if deeper.is_empty() {
                        item
                    } else {
                        walk(&item, deeper)
                    });
                }
                if deeper.is_empty() {
                    match key {
                        "index" => return Found::Value(Value::from(pass.index)),
                        "number" => return Found::Value(Value::from(pass.number)),
                        "count" => {
                            return Found::Value(
                                pass.count.map_or(Value::String(String::new()), Value::from),
                            )
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }
    Found::Unresolved
}

/// `resolveTemplateVars`: every token replaced by its text, or left as
/// written when its namespace is not this run's.
pub fn resolve(template: &str, context: Option<&Context>, outputs: Option<&StepOutputs>) -> String {
    if template.is_empty() || (context.is_none() && outputs.is_none()) {
        return template.to_owned();
    }
    TOKEN
        .replace_all(template, |caps: &regex::Captures<'_>| {
            match lookup(&caps[1], context, outputs) {
                Found::Unresolved => caps[0].to_owned(),
                Found::Value(v) => text_of(&v),
            }
        })
        .into_owned()
}

/// `resolveTemplateValue`: a template that is exactly one token yields the
/// value it names, a list staying a list and never cut; anything else is
/// text, as [`resolve`] gives it.
pub fn resolve_value(
    template: &str,
    context: Option<&Context>,
    outputs: Option<&StepOutputs>,
) -> Value {
    if context.is_some() || outputs.is_some() {
        if let Some(caps) = WHOLE.captures(template) {
            if let Found::Value(v) = lookup(&caps[1], context, outputs) {
                return if v.is_null() {
                    Value::String(String::new())
                } else {
                    v
                };
            }
        }
    }
    Value::String(resolve(template, context, outputs))
}

/// A `{{context.*}}` field: a path or a flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    Text(String),
    Flag(bool),
}

impl Field {
    pub fn text(&self) -> String {
        match self {
            Field::Text(s) => s.clone(),
            Field::Flag(b) => b.to_string(),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Field::Text(s) => Some(s),
            Field::Flag(_) => None,
        }
    }
}

/// `resolveContextField`: the task first, then the session that launched
/// the workflow. `None` when neither says.
pub fn context_field(field: &str, ctx: &Context) -> Option<Field> {
    let task = ctx.task.as_ref();
    let source = ctx.source.as_ref();
    // `??` passes over `undefined` and `null` only.
    let pick = |vals: &[Option<&Value>]| -> Option<Field> {
        vals.iter().find_map(|v| match v {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(Field::Text(s.clone())),
            Some(Value::Bool(b)) => Some(Field::Flag(*b)),
            Some(other) => Some(Field::Text(js::to_string(other))),
        })
    };
    fn of<'v>(record: Option<&'v Value>, key: &str) -> Option<&'v Value> {
        record.and_then(|r| r.get(key))
    }
    match field {
        "cwd" => pick(&[
            of(task, "worktreePath"),
            of(source, "worktreePath"),
            of(source, "projectPath"),
        ]),
        "projectPath" => pick(&[of(source, "projectPath")]),
        "projectName" => pick(&[of(task, "projectName"), of(source, "projectName")]),
        "branch" => pick(&[of(task, "branch"), of(source, "branch")]),
        "worktreePath" => pick(&[of(task, "worktreePath"), of(source, "worktreePath")]),
        "useWorktree" => {
            let has = |v: Option<&Value>, key: &str| crate::is_truthy(of(v, key));
            if task.is_some() {
                return pick(&[of(task, "useWorktree")])
                    .or(Some(Field::Flag(has(task, "worktreePath"))));
            }
            if source.is_some() {
                return pick(&[of(source, "isWorktree")])
                    .or(Some(Field::Flag(has(source, "worktreePath"))));
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LoopPass;
    use serde_json::json;

    fn ctx(v: Value) -> Context {
        Context::from_json(&v).unwrap()
    }

    #[test]
    fn resolves_each_namespace() {
        let c = ctx(json!({
            "task": { "id": "t", "title": "Fix", "order": 2.0, "images": ["a", "b"] },
            "trigger": { "type": "webhook", "body": { "n": [1, { "k": "v" }] }, "headers": { "content-type": "json" } },
            "connectorItem": { "externalId": "42" },
            "inputs": { "who": "me", "issue": { "number": 7 } },
            "source": { "projectPath": "/p", "worktreePath": "/w" }
        }));
        let mut outputs = StepOutputs::new();
        outputs.insert("plan".into(), json!({ "output": "ok", "items": [1, 2, 3] }));
        let r = |t: &str| resolve(t, Some(&c), Some(&outputs));
        assert_eq!(
            r("{{task.title}} #{{task.order}} {{task.images}}"),
            "Fix #2 a,b"
        );
        assert_eq!(
            r("{{trigger.body.n.1.k}} {{trigger.headers.content-type}}"),
            "v json"
        );
        assert_eq!(r("{{connectorItem.externalId}}"), "42");
        assert_eq!(
            r("{{inputs.who}} {{inputs.issue.number}} [{{inputs.none}}]"),
            "me 7 []"
        );
        assert_eq!(
            r("{{steps.plan.output}} {{steps.plan.items.length}}"),
            "ok 3"
        );
        assert_eq!(r("{{steps.plan.items}}"), "[1,2,3]");
        assert_eq!(r("[{{steps.missing.output}}]"), "[]");
        assert_eq!(r("{{context.cwd}} {{context.projectPath}}"), "/w /p");
        assert_eq!(
            r("{{loop.index}} {{nope.x}} {{ plain }}"),
            "{{loop.index}} {{nope.x}} {{ plain }}"
        );
    }

    #[test]
    fn a_whole_token_keeps_its_value() {
        let mut outputs = StepOutputs::new();
        outputs.insert("a".into(), json!({ "items": [{ "x": 1 }] }));
        assert_eq!(
            resolve_value(" {{steps.a.items}} ", None, Some(&outputs)),
            json!([{ "x": 1 }])
        );
        assert_eq!(
            resolve_value("{{steps.a.none}}", None, Some(&outputs)),
            json!("")
        );
        assert_eq!(
            resolve_value("x {{steps.a.items}}", None, Some(&outputs)),
            json!(r#"x [{"x":1}]"#)
        );
        assert_eq!(
            resolve_value("{{steps.a.items}}", None, None),
            json!("{{steps.a.items}}")
        );
    }

    #[test]
    fn a_loop_pass_names_its_item() {
        let c = Context {
            pass: Some(LoopPass {
                item: Some(json!({ "id": 9 })),
                index: 2,
                number: 3,
                count: None,
            }),
            ..Context::default()
        };
        assert_eq!(
            resolve(
                "{{loop.item.id}} {{loop.index}} {{loop.number}} [{{loop.count}}] {{loop.item}}",
                Some(&c),
                None
            ),
            r#"9 2 3 [] {"id":9}"#
        );
    }

    #[test]
    fn long_text_keeps_its_end() {
        let mut outputs = StepOutputs::new();
        let long = format!("{}END", "x".repeat(MAX_OUTPUT_LENGTH));
        outputs.insert("a".into(), json!({ "output": long }));
        let got = resolve("{{steps.a.output}}", None, Some(&outputs));
        assert_eq!(js::utf16_len(&got), MAX_OUTPUT_LENGTH);
        assert!(got.ends_with("END"));
    }

    #[test]
    fn context_fields_prefer_the_task() {
        let both = ctx(
            json!({ "task": { "projectName": "a", "useWorktree": false }, "source": { "projectName": "b", "isWorktree": true } }),
        );
        assert_eq!(
            context_field("projectName", &both),
            Some(Field::Text("a".into()))
        );
        assert_eq!(
            context_field("useWorktree", &both),
            Some(Field::Flag(false))
        );
        let source = ctx(json!({ "source": { "worktreePath": "/w" } }));
        assert_eq!(
            context_field("useWorktree", &source),
            Some(Field::Flag(true))
        );
        assert_eq!(context_field("useWorktree", &Context::default()), None);
        assert_eq!(context_field("odd", &source), None);
    }

    #[test]
    fn without_anything_to_resolve_against_the_text_is_kept() {
        assert_eq!(resolve("{{task.title}}", None, None), "{{task.title}}");
        assert_eq!(resolve("", Some(&Context::default()), None), "");
        assert_eq!(
            resolve("{{task.title}}", Some(&Context::default()), None),
            "{{task.title}}"
        );
    }
}

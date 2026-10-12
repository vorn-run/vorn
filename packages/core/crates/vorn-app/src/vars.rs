//! The `{{…}}` variables a step can read (`template-vars.ts`): the trigger's
//! and context's, the run inputs, the loop's, and each earlier step's
//! outputs with their last-run values; and a template's preview against
//! them.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::runs::s;

/// `TemplateVariable`'s category, in the picker's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Task,
    Trigger,
    Context,
    Inputs,
    Loop,
    ConnectorItem,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Task,
        Category::Trigger,
        Category::Context,
        Category::Inputs,
        Category::Loop,
        Category::ConnectorItem,
    ];

    /// `VAR_CATEGORY_GROUPS`: the group name and what it says.
    pub fn group(self) -> (&'static str, &'static str) {
        match self {
            Category::Task => ("Task", ""),
            Category::Trigger => ("Trigger", ""),
            Category::Context => ("Context", "Resolved from the launching card or terminal"),
            Category::Inputs => ("Run Inputs", "Entered when the run is started"),
            Category::Loop => ("Loop", "The pass this step is on"),
            Category::ConnectorItem => (
                "Connector Item",
                "The upstream item that triggered this run",
            ),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Category::Task => "task",
            Category::Trigger => "trigger",
            Category::Context => "context",
            Category::Inputs => "inputs",
            Category::Loop => "loop",
            Category::ConnectorItem => "connectorItem",
        }
    }
}

/// One variable the picker offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    pub key: String,
    pub label: String,
    pub category: Category,
}

fn v(key: &str, label: &str, category: Category) -> Var {
    Var {
        key: key.into(),
        label: label.into(),
        category,
    }
}

/// `TEMPLATE_VARIABLES`.
pub fn template_variables() -> Vec<Var> {
    use Category::*;
    vec![
        v("{{task.title}}", "Title", Task),
        v("{{task.description}}", "Description", Task),
        v("{{task.id}}", "ID", Task),
        v("{{task.status}}", "Status", Task),
        v("{{task.branch}}", "Branch", Task),
        v("{{task.projectName}}", "Project", Task),
        v("{{trigger.fromStatus}}", "Previous Status", Trigger),
        v("{{trigger.toStatus}}", "New Status", Trigger),
        v("{{trigger.restore}}", "Cold or Warm", Trigger),
        v("{{trigger.body}}", "Request Body", Trigger),
        v("{{trigger.headers}}", "Request Headers", Trigger),
        v("{{trigger.query}}", "Query Parameters", Trigger),
        v("{{connectorItem.externalId}}", "External ID", ConnectorItem),
        v("{{connectorItem.title}}", "Item Title", ConnectorItem),
        v("{{connectorItem.externalUrl}}", "Item URL", ConnectorItem),
        v("{{connectorItem.body}}", "Item Body", ConnectorItem),
        v("{{connectorItem.connectorId}}", "Connector", ConnectorItem),
        v("{{context.cwd}}", "Working Directory", Context),
        v("{{context.projectPath}}", "Project Path", Context),
        v("{{context.projectName}}", "Project Name", Context),
        v("{{context.branch}}", "Branch", Context),
        v("{{context.worktreePath}}", "Worktree Path", Context),
    ]
}

/// `isContextRef`: the whole value is one `{{context.x}}`.
pub fn is_context_ref(value: Option<&str>) -> bool {
    let Some(t) = value.map(str::trim) else {
        return false;
    };
    let Some(inner) = t.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")) else {
        return false;
    };
    let inner = inner.trim();
    inner
        .strip_prefix("context.")
        .is_some_and(|f| !f.is_empty() && f.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// `getAvailableContextVars`.
pub fn context_vars(trigger: Option<&str>, contextual: bool) -> Vec<Var> {
    let task_trigger = matches!(trigger, Some("taskCreated" | "taskStatusChanged"));
    template_variables()
        .into_iter()
        .filter(|var| {
            if task_trigger && var.category == Category::Task {
                return true;
            }
            if task_trigger
                && var.category == Category::Trigger
                && trigger == Some("taskStatusChanged")
            {
                return var.key.contains("Status");
            }
            if trigger == Some("sessionRestored") {
                if var.category == Category::Context {
                    return true;
                }
                if var.category == Category::Trigger {
                    return var.key.contains("trigger.restore");
                }
            }
            if trigger == Some("webhook") && var.category == Category::Trigger {
                return ["trigger.body", "trigger.headers", "trigger.query"]
                    .iter()
                    .any(|k| var.key.contains(k));
            }
            contextual && var.category == Category::Context
        })
        .collect()
}

/// `buildLoopVars`: the `{{loop.*}}` a step inside a loop, or the loop
/// itself, can read.
pub fn loop_vars(nodes: &[Value], node: Option<&str>) -> Vec<Var> {
    let Some(id) = node else {
        return Vec::new();
    };
    let Some(lp) = nodes.iter().find(|n| {
        s(n, "type") == Some("loop")
            && (s(n, "id") == Some(id)
                || n.pointer("/config/bodyNodeIds")
                    .and_then(Value::as_array)
                    .is_some_and(|b| b.iter().any(|x| x.as_str() == Some(id))))
    }) else {
        return Vec::new();
    };
    let each = lp.pointer("/config/mode").and_then(Value::as_str) == Some("forEach");
    let mut out = Vec::new();
    if each {
        out.push(v("{{loop.item}}", "This item", Category::Loop));
    }
    out.push(v(
        "{{loop.number}}",
        if each {
            "Item number, from 1"
        } else {
            "Pass number, from 1"
        },
        Category::Loop,
    ));
    out.push(v("{{loop.index}}", "Index, from 0", Category::Loop));
    out.push(v(
        "{{loop.count}}",
        if each {
            "How many items"
        } else {
            "Most passes"
        },
        Category::Loop,
    ));
    out
}

/// `buildInputVars`.
pub fn input_vars(inputs: &[Value]) -> Vec<Var> {
    inputs
        .iter()
        .filter_map(|i| {
            let key = s(i, "key").filter(|k| !k.is_empty())?;
            let label = s(i, "label").filter(|l| !l.is_empty()).unwrap_or(key);
            Some(v(&format!("{{{{inputs.{key}}}}}"), label, Category::Inputs))
        })
        .collect()
}

/// `getAncestorNodes`: every step upstream, nearest first, triggers out.
pub fn ancestors<'a>(nodes: &'a [Value], edges: &[Value], current: &str) -> Vec<&'a Value> {
    let mut preds: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in edges {
        if let (Some(a), Some(b)) = (s(e, "source"), s(e, "target")) {
            preds.entry(b).or_default().push(a);
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::from([current]);
    let mut queue = VecDeque::from([current]);
    while let Some(id) = queue.pop_front() {
        for p in preds.get(id).into_iter().flatten() {
            if !seen.insert(p) {
                continue;
            }
            queue.push_back(p);
            if let Some(n) = nodes.iter().find(|n| s(n, "id") == Some(p)) {
                if s(n, "type") != Some("trigger") {
                    out.push(n);
                }
            }
        }
    }
    out
}

/// One output key a step offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub key: String,
    pub label: String,
    pub description: String,
    /// What the last run put there, as one line.
    pub value: Option<String>,
}

/// `StepVariableGroup`: a step's outputs.
#[derive(Debug, Clone, PartialEq)]
pub struct StepGroup {
    pub node: String,
    pub label: String,
    pub slug: String,
    pub kind: String,
    pub keys: Vec<Key>,
    pub connection: Option<String>,
    pub run_status: Option<String>,
    pub run_completed: Option<String>,
    pub run_outputs: Option<serde_json::Map<String, Value>>,
}

fn k(key: &str, label: &str, description: &str) -> Key {
    Key {
        key: key.into(),
        label: label.into(),
        description: description.into(),
        value: None,
    }
}

fn defaults() -> Vec<Key> {
    vec![
        k("output", "Output", "Primary output (stdout / agent logs)"),
        k("status", "Status", "success or error"),
        k("error", "Error", "Error message if failed"),
    ]
}

fn schema_keys(schema: Option<&Value>) -> Vec<Key> {
    schema
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .map(|(key, raw)| {
                    let hint = match raw.get("type") {
                        Some(Value::String(t)) => t.clone(),
                        Some(Value::Array(ts)) => ts
                            .iter()
                            .filter_map(Value::as_str)
                            .find(|t| *t != "null")
                            .or_else(|| ts.first().and_then(Value::as_str))
                            .unwrap_or("")
                            .to_owned(),
                        _ => String::new(),
                    };
                    k(
                        key,
                        key,
                        s(raw, "description")
                            .filter(|d| !d.is_empty())
                            .unwrap_or(&hint),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `formatRunValue`: one line, cut at 60.
pub fn format_run_value(val: &Value) -> String {
    let cut = |t: String| {
        if t.chars().count() > 60 {
            format!("{}\u{2026}", t.chars().take(60).collect::<String>())
        } else {
            t
        }
    };
    match val {
        Value::String(t) => {
            let flat = t.split_whitespace().collect::<Vec<_>>().join(" ");
            Value::String(cut(flat)).to_string()
        }
        other => cut(other.to_string()),
    }
}

/// What the last run read out of each step (`buildStepOutputsMap`), and
/// how each step ended.
#[derive(Debug, Clone, Default)]
pub struct LastRun {
    pub outputs: HashMap<String, serde_json::Map<String, Value>>,
    pub states: HashMap<String, (String, Option<String>)>,
}

/// `buildStepOutputsMap` over the newest run.
pub fn last_run(run: &Value, nodes: &[Value]) -> LastRun {
    let mut out = LastRun::default();
    for ns in crate::runs::states(run) {
        let Some(nid) = s(ns, "nodeId") else { continue };
        if let Some(st) = s(ns, "status") {
            out.states.insert(
                nid.to_owned(),
                (st.to_owned(), s(ns, "completedAt").map(str::to_owned)),
            );
        }
        let Some(node) = nodes.iter().find(|n| s(n, "id") == Some(nid)) else {
            continue;
        };
        let Some(slug) = s(node, "slug") else {
            continue;
        };
        let settled = matches!(s(ns, "status"), Some("success" | "error"));
        let gate = (s(node, "type") == Some("approval"))
            .then(|| gate_outputs(ns))
            .flatten();
        if !settled && gate.is_none() {
            continue;
        }
        let mut map = serde_json::Map::new();
        if settled {
            if let Some(Value::Object(o)) = ns.get("structuredOutput") {
                map.extend(o.clone());
            }
            let output = s(ns, "output")
                .filter(|o| !o.is_empty())
                .or_else(|| s(ns, "logs"))
                .unwrap_or("");
            map.insert("output".into(), Value::String(output.into()));
            map.insert(
                "status".into(),
                Value::String(s(ns, "status").unwrap_or("").into()),
            );
            map.insert(
                "error".into(),
                Value::String(s(ns, "error").unwrap_or("").into()),
            );
            map.insert(
                "worktreePath".into(),
                Value::String(s(ns, "worktreePath").unwrap_or("").into()),
            );
        }
        if let Some(g) = gate {
            map.extend(g);
        }
        out.outputs.insert(slug.to_owned(), map);
    }
    out
}

fn gate_outputs(state: &Value) -> Option<serde_json::Map<String, Value>> {
    let entries = state
        .get("feedback")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if state.get("round").is_none_or(Value::is_null) && entries.is_empty() {
        return None;
    }
    let text = s(state, "editedText")
        .or_else(|| s(state, "editableText"))
        .unwrap_or("")
        .to_owned();
    let mut map = serde_json::Map::new();
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        if let Ok((items, _)) = crate::gates::item_list(&v) {
            if crate::gates::is_record_list(&items) {
                map.insert("items".into(), Value::Array(items));
            }
        }
    }
    map.insert("text".into(), Value::String(text));
    map.insert(
        "feedback".into(),
        Value::String(
            entries
                .last()
                .and_then(|e| s(e, "comment"))
                .unwrap_or("")
                .into(),
        ),
    );
    map.insert(
        "feedbackAll".into(),
        Value::String(
            entries
                .iter()
                .map(|e| {
                    format!(
                        "Round {}: {}",
                        crate::workflows::text_of(e.get("round")),
                        s(e, "comment").unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    );
    let comments: Vec<Value> = entries
        .last()
        .and_then(|e| e.get("comments"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            serde_json::json!({
                "quote": s(c, "quote").unwrap_or(""),
                "comment": s(c, "comment").unwrap_or(""),
                "anchored": s(c, "quote").is_some_and(|q| !q.is_empty()),
            })
        })
        .collect();
    map.insert("comments".into(), Value::Array(comments));
    map.insert(
        "round".into(),
        state
            .get("round")
            .cloned()
            .filter(|r| !r.is_null())
            .unwrap_or(Value::from(1)),
    );
    Some(map)
}

/// `buildStepGroups`: each upstream step's outputs, typed where a schema
/// says, with the last run's values.
pub fn step_groups(
    ancestors: &[&Value],
    actions: &HashMap<String, Vec<Value>>,
    last: Option<&LastRun>,
) -> Vec<StepGroup> {
    ancestors
        .iter()
        .filter_map(|n| {
            let slug = s(n, "slug").filter(|x| !x.is_empty())?.to_owned();
            let kind = s(n, "type").unwrap_or("").to_owned();
            let cfg = n.get("config").cloned().unwrap_or_default();
            let mut keys = defaults();
            match kind.as_str() {
                "callConnectorAction" => {
                    let schema = s(&cfg, "connectionId")
                        .zip(s(&cfg, "action"))
                        .and_then(|(c, a)| actions.get(c)?.iter().find(|x| s(x, "type") == Some(a)))
                        .and_then(|a| a.get("outputSchema"));
                    let sk = schema_keys(schema);
                    if !sk.is_empty() {
                        keys = [sk, keys].concat();
                    }
                }
                "httpRequest" => {
                    keys = [
                        vec![
                            k("status", "status", "HTTP status code"),
                            k("body", "body", "Response body, JSON-parsed when possible"),
                            k("headers", "headers", "Response headers"),
                        ],
                        keys,
                    ]
                    .concat();
                }
                "approval" => {
                    keys = [
                        vec![
                            k("items", "items", "The rows the reviewer kept, when the text is a JSON list"),
                            k("text", "text", "The text the gate showed, as the reviewer left it"),
                            k("comments", "comments", "Anchored comments from the latest round, as JSON: quote, comment, anchored"),
                            k("feedback", "feedback", "The general note from the latest round"),
                            k("feedbackAll", "feedbackAll", "Every note, one line per round"),
                            k("round", "round", "Which time the gate asked, from 1"),
                        ],
                        keys,
                    ]
                    .concat();
                }
                "loop" => {
                    keys = [
                        vec![
                            k("results", "results", "Each pass: its item, status and step outputs"),
                            k("outputs", "outputs", "Each pass's final output, in order"),
                            k("passes", "passes", "How many passes ran"),
                            k("count", "count", "Items in the list, or the most passes allowed"),
                        ],
                        keys,
                    ]
                    .concat();
                }
                "launchAgent" => {
                    let headless = cfg.get("headless").and_then(Value::as_bool) == Some(true);
                    let sk = if headless { schema_keys(cfg.get("outputSchema")) } else { Vec::new() };
                    keys = [sk, keys, vec![k("worktreePath", "Worktree", "Directory the step worked in")]].concat();
                }
                _ => {}
            }
            let id = s(n, "id").unwrap_or("").to_owned();
            let outputs = last.and_then(|l| l.outputs.get(&slug)).cloned();
            let state = last.and_then(|l| l.states.get(&id)).cloned();
            if let Some(o) = &outputs {
                let declared: HashSet<String> = keys.iter().map(|k| k.key.clone()).collect();
                let found: Vec<Key> = o
                    .keys()
                    .filter(|key| !declared.contains(*key))
                    .map(|key| k(key, key, "From the last run"))
                    .collect();
                keys = [found, keys].concat();
                for key in &mut keys {
                    key.value = o.get(&key.key).map(format_run_value).filter(|v| !v.is_empty());
                }
            }
            Some(StepGroup {
                node: id,
                label: s(n, "label").unwrap_or("").to_owned(),
                slug,
                connection: (kind == "callConnectorAction")
                    .then(|| s(&cfg, "connectionId").map(str::to_owned))
                    .flatten(),
                kind,
                keys,
                run_status: state.as_ref().map(|s| s.0.clone()),
                run_completed: state.and_then(|s| s.1),
                run_outputs: outputs,
            })
        })
        .collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            row[j] = (prev[j] + 1)
                .min(row[j - 1] + 1)
                .min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        prev = row;
    }
    prev[b.len()]
}

/// `suggestNearestPath`.
pub fn nearest(path: &str, candidates: &[String]) -> Option<String> {
    let mut best = None;
    let mut score = usize::MAX;
    for c in candidates {
        if c == path {
            continue;
        }
        let d = edit_distance(path, c);
        if d < score {
            score = d;
            best = Some(c.clone());
        }
    }
    (score <= 3.max(path.chars().count() / 3))
        .then_some(best)
        .flatten()
}

/// `TemplatePreview`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Preview {
    pub resolved: Option<String>,
    pub broken: Option<(String, Option<String>)>,
}

fn walk<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path {
        cur = cur.as_object()?.get(*seg)?;
    }
    Some(cur)
}

fn stringify(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(t) => t.clone(),
        other => other.to_string(),
    }
}

/// The `{{ steps.x.y }}` tokens of `template`, as `(start, end, path)`.
fn step_tokens(template: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = template[at..].find("{{") {
        let start = at + i;
        let Some(j) = template[start + 2..].find("}}") else {
            break;
        };
        let end = start + 2 + j + 2;
        let inner = template[start + 2..start + 2 + j].trim();
        if let Some(path) = inner.strip_prefix("steps.") {
            if !path.is_empty()
                && path
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
            {
                out.push((start, end, path.to_owned()));
            }
        }
        at = end;
    }
    out
}

/// `previewStepTokens`: known paths resolved to their last-run text, the
/// first unknown one reported with the nearest real path.
pub fn preview(template: &str, groups: &[StepGroup]) -> Preview {
    let tokens = step_tokens(template);
    if tokens.is_empty() {
        return Preview::default();
    }
    let known: Vec<String> = groups
        .iter()
        .flat_map(|g| {
            g.keys
                .iter()
                .map(move |k| format!("steps.{}.{}", g.slug, k.key))
        })
        .collect();
    let mut broken = None;
    let mut any = false;
    let mut out = String::new();
    let mut last = 0;
    for (start, end, path) in tokens {
        out.push_str(&template[last..start]);
        last = end;
        let whole = &template[start..end];
        let parts: Vec<&str> = path.split('.').collect();
        let (slug, rest) = (parts[0], &parts[1..]);
        let group = groups.iter().find(|g| g.slug == slug);
        let report = |broken: &mut Option<(String, Option<String>)>| {
            if broken.is_none() {
                let token = format!("steps.{path}");
                *broken = Some((token.clone(), nearest(&token, &known)));
            }
        };
        let Some(g) = group.filter(|_| !rest.is_empty()) else {
            report(&mut broken);
            out.push_str(whole);
            continue;
        };
        let Some(outputs) = &g.run_outputs else {
            out.push_str(whole);
            continue;
        };
        let root = Value::Object(outputs.clone());
        match walk(&root, rest) {
            Some(v) => {
                any = true;
                out.push_str(&stringify(v));
            }
            None => {
                let exact = known.contains(&format!("steps.{path}"));
                let declared = known.contains(&format!("steps.{slug}.{}", rest[0]));
                let emitted = outputs.contains_key(rest[0]);
                if !exact && !(declared && !emitted) {
                    report(&mut broken);
                }
                out.push_str(whole);
            }
        }
    }
    out.push_str(&template[last..]);
    if broken.is_some() {
        return Preview {
            broken,
            ..Preview::default()
        };
    }
    Preview {
        resolved: any.then_some(out),
        broken: None,
    }
}

/// One line of the `{{` dropdown.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub group: String,
    pub group_id: String,
    pub key: String,
    pub label: String,
    pub description: String,
    pub pattern: String,
    pub value: Option<String>,
}

/// Every variable the dropdown offers, steps first, then each category.
pub fn items(groups: &[StepGroup], vars: &[Var]) -> Vec<Item> {
    let mut out = Vec::new();
    for g in groups {
        for k in &g.keys {
            out.push(Item {
                group: g.label.clone(),
                group_id: g.slug.clone(),
                key: k.key.clone(),
                label: k.label.clone(),
                description: k.description.clone(),
                pattern: format!("{{{{steps.{}.{}}}}}", g.slug, k.key),
                value: k.value.clone(),
            });
        }
    }
    for c in Category::ALL {
        let (group, description) = c.group();
        for var in vars.iter().filter(|v| v.category == c) {
            out.push(Item {
                group: group.into(),
                group_id: c.id().into(),
                key: var.key.clone(),
                label: var.label.clone(),
                description: description.into(),
                pattern: var.key.clone(),
                value: None,
            });
        }
    }
    out
}

/// What the filter keeps (label, group or key containing it).
pub fn filtered(all: Vec<Item>, filter: &str) -> Vec<Item> {
    if filter.is_empty() {
        return all;
    }
    let f = filter.to_lowercase();
    all.into_iter()
        .filter(|i| {
            i.label.to_lowercase().contains(&f)
                || i.group.to_lowercase().contains(&f)
                || i.key.to_lowercase().contains(&f)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_trigger_decides_the_context_variables() {
        let keys = |vs: Vec<Var>| vs.into_iter().map(|v| v.key).collect::<Vec<_>>();
        assert!(context_vars(Some("manual"), false).is_empty());
        assert_eq!(context_vars(Some("manual"), true).len(), 5);
        let moved = keys(context_vars(Some("taskStatusChanged"), false));
        assert!(
            moved.contains(&"{{trigger.toStatus}}".to_owned())
                && moved.contains(&"{{task.title}}".to_owned())
        );
        assert_eq!(keys(context_vars(Some("webhook"), false)).len(), 3);
        assert!(
            is_context_ref(Some(" {{context.cwd}} ")) && !is_context_ref(Some("x {{context.cwd}}"))
        );
    }

    #[test]
    fn steps_offer_their_outputs_with_last_run_values() {
        let nodes = vec![
            json!({ "id": "a", "type": "script", "slug": "fetch", "label": "Fetch" }),
            json!({ "id": "b", "type": "httpRequest", "slug": "post", "label": "Post" }),
            json!({ "id": "c", "type": "script", "slug": "use", "label": "Use" }),
        ];
        let edges = vec![
            json!({ "source": "a", "target": "b" }),
            json!({ "source": "b", "target": "c" }),
        ];
        let up = ancestors(&nodes, &edges, "c");
        assert_eq!(
            up.iter().map(|n| s(n, "id").unwrap()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        let run = json!({ "nodeStates": [{ "nodeId": "a", "status": "success", "output": "hello  world", "structuredOutput": { "n": 2 } }] });
        let last = last_run(&run, &nodes);
        let groups = step_groups(&up, &HashMap::new(), Some(&last));
        assert_eq!(groups[0].keys[0].key, "status", "a request's keys first");
        let fetch = &groups[1];
        assert_eq!(fetch.keys[0].key, "n", "found in the run");
        assert_eq!(
            fetch
                .keys
                .iter()
                .find(|k| k.key == "output")
                .unwrap()
                .value
                .as_deref(),
            Some("\"hello world\"")
        );
        let p = preview("say {{steps.fetch.output}}", &groups);
        assert_eq!(p.resolved.as_deref(), Some("say hello  world"));
        let p = preview("{{steps.fetch.outptu}}", &groups);
        assert_eq!(
            p.broken,
            Some((
                "steps.fetch.outptu".into(),
                Some("steps.fetch.output".into())
            ))
        );
    }

    #[test]
    fn a_loop_and_its_body_read_the_pass() {
        let nodes = vec![
            json!({ "id": "l", "type": "loop", "config": { "mode": "forEach", "bodyNodeIds": ["x"] } }),
        ];
        assert_eq!(loop_vars(&nodes, Some("x")).len(), 4);
        assert_eq!(loop_vars(&nodes, Some("l"))[0].key, "{{loop.item}}");
        assert!(loop_vars(&nodes, Some("y")).is_empty());
        let ins = input_vars(&[json!({ "key": "who", "label": "" })]);
        assert_eq!(
            (ins[0].key.as_str(), ins[0].label.as_str()),
            ("{{inputs.who}}", "who")
        );
    }
}

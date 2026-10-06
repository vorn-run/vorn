//! The refinements the tools' schemas use, each a rule zod runs as code.
//!
//! `scripts/gen-mcp-tools.mjs` names every one it finds and refuses one it
//! does not know, so each name here answers for one function in the
//! TypeScript, with that function's messages word for word.

use std::sync::LazyLock;

use serde_json::Value;

use super::{Issue, Registry, Seg};
use crate::json;

/// One refinement, by the name the generator gave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refinement {
    /// `V.name`: no `..`, `/` or `\`.
    SafeName,
    /// `V.absolutePath`: starts with `/`.
    AbsolutePath,
    /// An `outputSchema` only on a headless agent.
    OutputSchemaNeedsHeadless,
    /// A run input that can be satisfied: a select with options, a default
    /// that fits its type.
    WorkflowInputDef,
    /// No two run inputs under one key.
    UniqueInputKeys,
    /// A loop that says what it walks or how often it repeats.
    LoopConfig,
    /// A node's config checked against its type's schema.
    NodeConfig,
}

impl Refinement {
    /// The refinement the generator wrote as `name`.
    pub fn named(name: &str) -> Option<Refinement> {
        Some(match name {
            "safeName" => Refinement::SafeName,
            "absolutePath" => Refinement::AbsolutePath,
            "outputSchemaNeedsHeadless" => Refinement::OutputSchemaNeedsHeadless,
            "workflowInputDef" => Refinement::WorkflowInputDef,
            "uniqueInputKeys" => Refinement::UniqueInputKeys,
            "loopConfig" => Refinement::LoopConfig,
            "nodeConfig" => Refinement::NodeConfig,
            _ => return None,
        })
    }
}

/// `MAX_LOOP_ITERATIONS`, as the generated node reference records it.
pub(crate) static MAX_LOOP_ITERATIONS: LazyLock<f64> = LazyLock::new(|| {
    crate::workflow_nodes()
        .pointer("/limits/repeatMaxIterations")
        .and_then(Value::as_f64)
        .unwrap_or(10.0)
});

/// The issues `refinement` finds in `value`, which has already parsed.
///
/// A `.refine` issue sits at the check's own `path` with its own message and
/// lets later checks run; so does a `superRefine` issue, which names its own path.
pub(super) fn apply(
    registry: &Registry,
    refinement: Refinement,
    message: Option<&str>,
    path: &[Seg],
    value: &Value,
) -> Vec<Issue> {
    let refined = |ok: bool| -> Vec<Issue> {
        if ok {
            Vec::new()
        } else {
            vec![Issue::custom(
                path.to_vec(),
                message.unwrap_or("Invalid input").to_owned(),
            )]
        }
    };
    let added = |path: Vec<Seg>, message: String| Issue::custom(path, message);
    match refinement {
        Refinement::SafeName => {
            let s = value.as_str().unwrap_or_default();
            refined(!s.contains("..") && !s.contains('/') && !s.contains('\\'))
        }
        Refinement::AbsolutePath => refined(value.as_str().unwrap_or_default().starts_with('/')),
        Refinement::OutputSchemaNeedsHeadless => refined(
            !json::truthy(value.get("outputSchema"))
                || value.get("headless") == Some(&Value::Bool(true)),
        ),
        Refinement::WorkflowInputDef => input_def(value)
            .into_iter()
            .map(|(key, message)| added(vec![Seg::Key(key.into())], message))
            .collect(),
        Refinement::UniqueInputKeys => {
            let mut seen: Vec<&str> = Vec::new();
            let mut issues = Vec::new();
            for (index, def) in value
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let key = def.get("key").and_then(Value::as_str).unwrap_or_default();
                if seen.contains(&key) {
                    issues.push(added(
                        vec![Seg::Index(index), Seg::Key("key".into())],
                        format!(
                            "duplicate input key \"{key}\" — only one value can survive under {{{{inputs.{key}}}}}"
                        ),
                    ));
                }
                seen.push(key);
            }
            issues
        }
        Refinement::LoopConfig => loop_config(value)
            .into_iter()
            .map(|(key, message)| added(vec![Seg::Key(key.into())], message))
            .collect(),
        Refinement::NodeConfig => {
            let kind = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            node_config_issues(registry, kind, value.get("config"))
                .into_iter()
                .map(|issue| {
                    let mut path = vec![Seg::Key("config".into())];
                    path.extend(issue.path);
                    added(path, issue.message)
                })
                .collect()
        }
    }
}

/// `nodeConfigIssues(type, config)`: what the type's config schema finds
/// wrong, nothing for a type it does not know (the node's own `type` check
/// reports that).
pub fn node_config_issues(registry: &Registry, kind: &str, config: Option<&Value>) -> Vec<Issue> {
    match registry.def(&format!("nodeConfig.{kind}")) {
        Some(schema) => registry.parse(schema, config).err().unwrap_or_default(),
        None => Vec::new(),
    }
}

/// workflowInputDefSchema's superRefine, as (field, message) pairs.
fn input_def(def: &Value) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let kind = def.get("type").and_then(Value::as_str).unwrap_or_default();
    let key = json::display(def.get("key"));
    let options = def
        .get("options")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if kind == "select" && options.is_empty() {
        out.push((
            "options",
            format!(
                "select input \"{key}\" declares no options, so the run dialog could offer nothing"
            ),
        ));
    }
    let Some(default) = def.get("defaultValue") else {
        return out;
    };
    let shown = json::display(Some(default));
    if kind == "number" && !json::to_number(Some(default)).is_finite() {
        out.push((
            "defaultValue",
            format!("default \"{shown}\" for number input \"{key}\" is not a finite number"),
        ));
    }
    if kind == "boolean" && !matches!(default.as_str(), Some("true" | "false")) {
        out.push((
            "defaultValue",
            format!(
                "default \"{shown}\" for boolean input \"{key}\" must be \"true\" or \"false\""
            ),
        ));
    }
    if kind == "select"
        && !options.is_empty()
        && !options
            .iter()
            .any(|o| json::strict_equals(o.get("value"), Some(default)))
    {
        out.push((
            "defaultValue",
            format!("default \"{shown}\" for select input \"{key}\" is not one of its options"),
        ));
    }
    out
}

/// The loop config's superRefine, as (field, message) pairs.
fn loop_config(config: &Value) -> Vec<(&'static str, String)> {
    if config.get("mode").and_then(Value::as_str) == Some("forEach") {
        let items = config.get("items").and_then(Value::as_str).map(json::trim);
        if !items.is_some_and(|s| !s.is_empty()) {
            return vec![(
                "items",
                "a forEach loop needs items: a template naming the list to walk".to_owned(),
            )];
        }
        return Vec::new();
    }
    let max = *MAX_LOOP_ITERATIONS;
    let ok = match config.get("maxIterations").and_then(Value::as_f64) {
        Some(n) => n.fract() == 0.0 && n >= 1.0 && n <= max,
        None => false,
    };
    if ok {
        Vec::new()
    } else {
        vec![(
            "maxIterations",
            format!(
                "a repeat loop needs maxIterations as a whole number from 1 to {}",
                json::number_to_string(max)
            ),
        )]
    }
}

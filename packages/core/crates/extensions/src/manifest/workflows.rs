//! What an extension adds to workflows: node types (steps a workflow can
//! use, which the extension runs), trigger types (what starts one, which
//! the extension fires or the core times), and workflows it ships as KDL
//! files. Read from `manifest.json`'s `nodes`, `triggers` and `workflows`,
//! as `[[contributes.nodes]]`, `[[contributes.triggers]]` and
//! `[[contributes.workflows]]` write them.

use serde::Serialize;
use serde_json::{Map, Value};

use super::{contribution, Contribution};
use crate::js;

/// A node type: a step a workflow can use, which its extension runs when a
/// run reaches it (`guest.handle("workflow.node.<id>")`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeDecl {
    /// `id` is the step's `type` in a workflow.
    #[serde(flatten)]
    pub base: Contribution,
    /// A kit icon by name, as the step library draws it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The step library's group it shows in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// A JSON Schema of its config, `nodes/<name>.json` in the package: the
    /// editor draws its form from it and agents read it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    /// What later steps read of it, as `{{steps.<slug>.<field>}}`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<NodeOutput>,
    /// What an author must know, a sentence each.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
}

/// One output of a node type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeOutput {
    pub field: String,
    pub description: String,
}

/// When the core fires a trigger type itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerTiming {
    /// On the config's `cron`.
    Cron,
    /// Once, at the config's `runAt`.
    Once,
    /// By a request to the workflow's webhook address.
    Webhook,
}

impl TriggerTiming {
    fn parse(text: &str) -> Option<TriggerTiming> {
        match text {
            "cron" => Some(TriggerTiming::Cron),
            "once" => Some(TriggerTiming::Once),
            "webhook" => Some(TriggerTiming::Webhook),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TriggerTiming::Cron => "cron",
            TriggerTiming::Once => "once",
            TriggerTiming::Webhook => "webhook",
        }
    }
}

/// A trigger type: what starts a workflow. Its extension fires it with
/// `workflow.fire`, or the core does on its `timing`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TriggerDecl {
    /// `id` is the trigger's `triggerType` in a workflow.
    #[serde(flatten)]
    pub base: Contribution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<TriggerTiming>,
    /// Each fire is a poll the extension fans out into items, so a person's
    /// Run asks for one rather than starting a run.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub poll: bool,
    /// A JSON Schema of its config, `nodes/<name>.json` in the package.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
}

/// A workflow the extension ships: `workflows/<name>.toml` in the package,
/// added once when the extension is installed or switched on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowDecl {
    #[serde(flatten)]
    pub base: Contribution,
    pub file: String,
}

/// `nodes/<name>.json`, never climbing out of the package.
pub fn is_schema_path(path: &str) -> bool {
    in_folder(path, "nodes/", ".json")
}

/// `workflows/<name>.toml`, never climbing out of the package.
pub fn is_workflow_path(path: &str) -> bool {
    in_folder(path, "workflows/", ".toml")
}

fn in_folder(path: &str, folder: &str, ext: &str) -> bool {
    let Some(rest) = path.strip_prefix(folder).and_then(|r| r.strip_suffix(ext)) else {
        return false;
    };
    !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
        && !path
            .split('/')
            .any(|s| s == ".." || s == "." || s.is_empty())
}

fn icon(raw: &Map<String, Value>) -> Option<String> {
    let name = js::trim(js::str_of(raw.get("icon")));
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-');
    ok.then(|| name.to_owned())
}

fn short(raw: &Map<String, Value>, key: &str) -> Option<String> {
    let text = js::trim(js::str_of(raw.get(key)));
    (!text.is_empty()).then(|| js::slice16(text, 120).to_owned())
}

/// Why a node, trigger or workflow entry written as `raw` would be dropped:
/// the field and what is wrong.
pub fn problem(kind: &str, raw: &Map<String, Value>) -> Option<(&'static str, String)> {
    if contribution(raw).is_none() {
        return Some((
            "id",
            "needs an id of letters, digits, - and _, starting with a letter".into(),
        ));
    }
    let path = |key: &str| js::trim(js::str_of(raw.get(key))).to_owned();
    match kind {
        "workflows" if !is_workflow_path(&path("file")) => Some((
            "file",
            "is not a file under workflows/ ending in .toml".into(),
        )),
        "nodes" | "triggers" if raw.get("config").is_some() && !is_schema_path(&path("config")) => {
            Some((
                "config",
                "is not a JSON Schema under nodes/ ending in .json".into(),
            ))
        }
        "triggers"
            if raw.get("timing").is_some()
                && TriggerTiming::parse(js::str_of(raw.get("timing"))).is_none() =>
        {
            Some(("timing", "is one of cron, once and webhook".into()))
        }
        _ if raw.get("icon").is_some() && icon(raw).is_none() => {
            Some(("icon", "is not an icon name such as trash-2".into()))
        }
        _ => None,
    }
}

/// The node types of `manifest.json`'s `nodes`, those that read.
pub fn nodes<'a>(declared: impl Iterator<Item = &'a Map<String, Value>>) -> Vec<NodeDecl> {
    declared
        .filter(|raw| problem("nodes", raw).is_none())
        .filter_map(|raw| {
            let outputs = raw
                .get("outputs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .filter_map(|o| {
                    let field = js::trim(js::str_of(o.get("field")));
                    (!field.is_empty()).then(|| NodeOutput {
                        field: field.to_owned(),
                        description: js::str_of(o.get("description")).to_owned(),
                    })
                })
                .collect();
            let rules = raw
                .get("rules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            Some(NodeDecl {
                base: contribution(raw)?,
                icon: icon(raw),
                category: short(raw, "category"),
                config: short(raw, "config"),
                outputs,
                rules,
            })
        })
        .collect()
}

/// The trigger types of `manifest.json`'s `triggers`, those that read.
pub fn triggers<'a>(declared: impl Iterator<Item = &'a Map<String, Value>>) -> Vec<TriggerDecl> {
    declared
        .filter(|raw| problem("triggers", raw).is_none())
        .filter_map(|raw| {
            Some(TriggerDecl {
                base: contribution(raw)?,
                icon: icon(raw),
                timing: raw
                    .get("timing")
                    .and_then(Value::as_str)
                    .and_then(TriggerTiming::parse),
                poll: raw.get("poll").and_then(Value::as_bool) == Some(true),
                config: short(raw, "config"),
            })
        })
        .collect()
}

/// The workflows of `manifest.json`'s `workflows`, those that read.
pub fn workflows<'a>(declared: impl Iterator<Item = &'a Map<String, Value>>) -> Vec<WorkflowDecl> {
    declared
        .filter(|raw| problem("workflows", raw).is_none())
        .filter_map(|raw| {
            Some(WorkflowDecl {
                base: contribution(raw)?,
                file: js::trim(js::str_of(raw.get("file"))).to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_stay_in_their_folder() {
        assert!(is_workflow_path("workflows/review.toml"));
        assert!(is_workflow_path("workflows/team/review.toml"));
        assert!(!is_workflow_path("workflows/../x.toml"));
        assert!(!is_workflow_path("workflows/.toml"));
        assert!(!is_workflow_path("review.toml"));
        assert!(is_schema_path("nodes/deploy.json"));
        assert!(!is_schema_path("nodes/deploy.kdl"));
    }
}

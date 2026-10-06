//! Making a workflow portable and putting it back, from
//! `@vornrun/shared/workflow-portability`.
//!
//! Export rewrites this machine's paths to placeholders and drops its
//! connection ids, recording what each stood for; import resolves the
//! placeholders against a project and rebinds a connection where this machine
//! has one unambiguous answer.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::json;
use crate::tools::connection_connector_id;

pub const PROJECT_PATH_TOKEN: &str = "{{project.path}}";
pub const PROJECT_NAME_TOKEN: &str = "{{project.name}}";
pub const PORTABLE_FORMAT_VERSION: f64 = 1.0;
const HTTP_PROFILE_CONNECTOR: &str = "http";

/// Config keys a step may simply not have.
fn optional_key(key: &str) -> bool {
    key == "profileConnectionId" || key == "secretsFrom"
}

/// `{ ...value }`: an object's own properties, an array's or a string's
/// indexes, and nothing for anything else.
pub fn spread(value: Option<&Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(map)) => map.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v.clone()))
            .collect(),
        Some(Value::String(s)) => s
            .chars()
            .enumerate()
            .map(|(i, c)| (i.to_string(), Value::from(c.to_string())))
            .collect(),
        _ => Map::new(),
    }
}

/// `{ ...node, config }`.
fn with_config(node: &Value, config: Map<String, Value>) -> Value {
    let mut out = spread(Some(node));
    out.insert("config".into(), Value::Object(config));
    Value::Object(out)
}

/// `slugify(name)`.
pub fn slugify(name: &str) -> String {
    let lowered = name.to_lowercase();
    let mut out = String::new();
    let mut gap = false;
    for c in lowered.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    // A run at either end never pushed its dash, which is the trim; the cut
    // comes after it, so a slug can still end in one.
    let slug: String = out.chars().take(60).collect();
    if slug.is_empty() {
        "workflow".to_owned()
    } else {
        slug
    }
}

/// `slugify(value)` for a value from a file, which JavaScript only lowers if
/// it is a string.
pub fn slugify_value(value: Option<&Value>) -> Result<String, String> {
    match value {
        Some(Value::String(s)) => Ok(slugify(s)),
        None => Err("Cannot read properties of undefined (reading 'toLowerCase')".to_owned()),
        Some(Value::Null) => {
            Err("Cannot read properties of null (reading 'toLowerCase')".to_owned())
        }
        Some(_) => Err("name.toLowerCase is not a function".to_owned()),
    }
}

/// `importedWorkflowId(bundle, slug)`.
pub fn imported_workflow_id(bundle: &str, slug: &str) -> String {
    format!("import:{bundle}:{slug}")
}

/// `importedWorkflowIdFor`: the derived id, stepping aside from one already
/// held by a workflow of another name.
pub fn imported_workflow_id_for(
    bundle: &str,
    slug: &str,
    name: Option<&Value>,
    existing: &[Value],
) -> String {
    let mut attempt = 1;
    loop {
        let id = if attempt == 1 {
            imported_workflow_id(bundle, slug)
        } else {
            imported_workflow_id(bundle, &format!("{slug}-{attempt}"))
        };
        let held = existing
            .iter()
            .find(|w| w.get("id").and_then(Value::as_str) == Some(id.as_str()));
        match held {
            None => return id,
            Some(w) if json::strict_equals(w.get("name"), name) => return id,
            Some(_) => attempt += 1,
        }
    }
}

/// The connector a connection belongs to, packaged connectors included.
fn connector_of(connection: &Value) -> Option<Value> {
    let filters = match connection.get("filters") {
        None | Some(Value::Null) => json!({}),
        Some(f) => f.clone(),
    };
    let shaped = json!({ "connectorId": connection.get("connectorId"), "filters": filters });
    let shaped = match connection.get("connectorId") {
        Some(_) => shaped,
        None => json!({ "filters": filters }),
    };
    connection_connector_id(&shaped).ok().flatten()
}

/// `boundConnectionKey(node, config)`.
fn bound_connection_key(node: &Value, config: &Map<String, Value>) -> Option<&'static str> {
    match node.get("type").and_then(Value::as_str) {
        Some("trigger")
            if config.get("triggerType").and_then(Value::as_str) == Some("connectorPoll") =>
        {
            Some("connectionId")
        }
        Some("callConnectorAction") => Some("connectionId"),
        Some("httpRequest") => Some("profileConnectionId"),
        Some("script") => Some("secretsFrom"),
        _ => None,
    }
}

/// `resolveRequirement`: the connection a requirement binds to here, when
/// this machine has exactly one answer.
fn resolve_requirement(requirement: &Value, connections: &[Value]) -> Option<Value> {
    let http = requirement.get("kind").and_then(Value::as_str) == Some("httpProfile");
    let wanted = requirement.get("connectorId");
    let candidates: Vec<&Value> = connections
        .iter()
        .filter(|c| {
            let of = connector_of(c);
            if http {
                of.as_ref().and_then(Value::as_str) == Some(HTTP_PROFILE_CONNECTOR)
            } else {
                wanted.and_then(Value::as_str) != Some("")
                    && json::strict_equals(of.as_ref(), wanted)
            }
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let name = requirement.get("name");
    if name.and_then(Value::as_str) != Some("") {
        let named: Vec<&&Value> = candidates
            .iter()
            .filter(|c| json::strict_equals(c.get("name"), name))
            .collect();
        if let [one] = named.as_slice() {
            return Some(one.get("id").cloned().unwrap_or(Value::Null));
        }
    }
    match candidates.as_slice() {
        [one] => Some(one.get("id").cloned().unwrap_or(Value::Null)),
        _ => None,
    }
}

fn normalize_for_compare(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_owned()
}

fn replace_path(value: &str, project_path: &str) -> String {
    let v = normalize_for_compare(value);
    let root = normalize_for_compare(project_path);
    if root.is_empty() {
        return value.to_owned();
    }
    if v == root {
        return PROJECT_PATH_TOKEN.to_owned();
    }
    match v.strip_prefix(&format!("{root}/")) {
        Some(tail) => format!("{PROJECT_PATH_TOKEN}/{tail}"),
        None => value.to_owned(),
    }
}

/// `toPortable(workflow, projectPath, connections)`.
pub fn to_portable(workflow: &Value, project_path: &str, connections: &[Value]) -> Value {
    let slug = slugify(
        workflow
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let mut requires: Vec<Value> = Vec::new();

    let nodes: Vec<Value> = crate::tools::items(workflow.get("nodes"))
        .iter()
        .map(|node| {
            let mut config = spread(node.get("config"));
            let kind = node.get("type").and_then(Value::as_str);

            if kind == Some("trigger")
                && config.get("triggerType").and_then(Value::as_str) == Some("webhook")
            {
                config.insert("token".into(), json!(""));
            }
            if matches!(kind, Some("launchAgent" | "script")) {
                for key in ["projectPath", "cwd", "existingWorktreePath"] {
                    if let Some(Value::String(value)) = config.get(key) {
                        if !value.is_empty() {
                            let replaced = replace_path(value, project_path);
                            config.insert(key.into(), Value::from(replaced));
                        }
                    }
                }
                if matches!(config.get("projectName"), Some(Value::String(s)) if !s.is_empty()) {
                    config.insert("projectName".into(), json!(PROJECT_NAME_TOKEN));
                }
                config.shift_remove("remoteHostId");
            }

            if let Some(key) = bound_connection_key(node, &config) {
                let bound = config.get(key).cloned();
                let unbound =
                    !optional_key(key) && bound.as_ref().and_then(Value::as_str) == Some("");
                let named = matches!(&bound, Some(Value::String(s)) if !s.is_empty());
                if named || unbound {
                    let source = connections
                        .iter()
                        .find(|c| json::strict_equals(c.get("id"), bound.as_ref()));
                    let source_name = source
                        .and_then(|s| s.get("name"))
                        .filter(|n| !n.is_null())
                        .cloned()
                        .unwrap_or_else(|| json!(""));
                    let requirement = if key == "profileConnectionId" {
                        crate::tools::object([
                            ("kind", Some(json!("httpProfile"))),
                            ("nodeId", node.get("id").cloned()),
                            ("name", Some(source_name)),
                        ])
                    } else {
                        let connector = match source {
                            Some(s) => connector_of(s),
                            None => match config.get("connectorId") {
                                Some(Value::String(d)) => Some(Value::from(d.as_str())),
                                _ => Some(json!("")),
                            },
                        };
                        crate::tools::object([
                            ("kind", Some(json!("connection"))),
                            ("nodeId", node.get("id").cloned()),
                            ("connectorId", connector),
                            ("name", Some(source_name)),
                            (
                                "event",
                                config
                                    .get("event")
                                    .filter(|e| matches!(e, Value::String(s) if !s.is_empty()))
                                    .cloned(),
                            ),
                            ("key", (key == "secretsFrom").then(|| json!(key))),
                        ])
                    };
                    requires.push(requirement);
                    if optional_key(key) {
                        config.shift_remove(key);
                    } else {
                        config.insert(key.into(), json!(""));
                    }
                }
            }
            with_config(node, config)
        })
        .collect();

    crate::tools::object([
        ("version", Some(json::num(PORTABLE_FORMAT_VERSION))),
        ("slug", Some(Value::from(slug))),
        ("name", workflow.get("name").cloned()),
        (
            "icon",
            workflow
                .get("icon")
                .filter(|v| json::truthy(Some(v)))
                .cloned(),
        ),
        (
            "iconColor",
            workflow
                .get("iconColor")
                .filter(|v| json::truthy(Some(v)))
                .cloned(),
        ),
        ("staggerDelayMs", workflow.get("staggerDelayMs").cloned()),
        (
            "requires",
            (!requires.is_empty()).then_some(Value::Array(requires)),
        ),
        ("nodes", Some(Value::Array(nodes))),
        ("edges", workflow.get("edges").cloned()),
    ])
}

/// `bindsOnlyByHand`: a script's key is handed over by a person, in the step.
fn binds_only_by_hand(requirement: &Value) -> bool {
    requirement.get("kind").and_then(Value::as_str) == Some("connection")
        && requirement.get("key").and_then(Value::as_str) == Some("secretsFrom")
}

/// `unresolvedRequirements`: which requirements this machine cannot answer itself.
pub fn unresolved_requirements(portable: &Value, connections: &[Value]) -> Vec<Value> {
    let nodes = crate::tools::items(portable.get("nodes"));
    crate::tools::items(portable.get("requires"))
        .iter()
        .filter(|r| {
            nodes
                .iter()
                .any(|n| json::strict_equals(n.get("id"), r.get("nodeId")))
                && (binds_only_by_hand(r) || resolve_requirement(r, connections).is_none())
        })
        .cloned()
        .collect()
}

/// `fromPortable(portable, bundle, project, connections)`.
pub fn from_portable(
    portable: &Value,
    bundle: &str,
    project_name: &Value,
    project_path: &str,
    connections: &[Value],
) -> Value {
    let requires = crate::tools::items(portable.get("requires"));
    let root = project_path.trim_end_matches(['/', '\\']);
    let name_text = json::display(Some(project_name));

    let nodes: Vec<Value> = crate::tools::items(portable.get("nodes"))
        .iter()
        .map(|node| {
            let mut config = spread(node.get("config"));
            let key = bound_connection_key(node, &config);
            if let Some(key) = key.filter(|k| optional_key(k)) {
                config.shift_remove(key);
            }
            for value in config.values_mut() {
                if let Value::String(s) = value {
                    *s = s
                        .replace(PROJECT_PATH_TOKEN, root)
                        .replace(PROJECT_NAME_TOKEN, &name_text);
                }
            }
            for requirement in requires
                .iter()
                .filter(|r| json::strict_equals(r.get("nodeId"), node.get("id")))
            {
                let Some(key) = key else { continue };
                if binds_only_by_hand(requirement) {
                    continue;
                }
                if let Some(resolved) = resolve_requirement(requirement, connections) {
                    config.insert(key.into(), resolved);
                }
            }
            if node.get("type").and_then(Value::as_str) == Some("trigger")
                && config.get("triggerType").and_then(Value::as_str) == Some("webhook")
                && !json::truthy(config.get("token"))
            {
                config.insert(
                    "token".into(),
                    Value::from(uuid::Uuid::new_v4().to_string()),
                );
            }
            with_config(node, config)
        })
        .collect();

    let or = |key: &str, fallback: &str| match portable.get(key) {
        None | Some(Value::Null) => Value::from(fallback),
        Some(v) => v.clone(),
    };
    crate::tools::object([
        (
            "id",
            Some(Value::from(imported_workflow_id(
                bundle,
                &json::display(portable.get("slug")),
            ))),
        ),
        ("name", portable.get("name").cloned()),
        ("icon", Some(or("icon", "Zap"))),
        ("iconColor", Some(or("iconColor", "#6366f1"))),
        ("enabled", Some(Value::Bool(false))),
        ("staggerDelayMs", portable.get("staggerDelayMs").cloned()),
        ("nodes", Some(Value::Array(nodes))),
        ("edges", portable.get("edges").cloned()),
    ])
}

static MACHINE_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(^|["'\s\x{feff}])(/(Users|home)/|[A-Za-z]:[\\/]|\\\\[^\\/\s\x{feff}]+[\\/])"#)
        .expect("MACHINE_PATH is a valid pattern")
});

/// `residualAbsolutePaths`: what is still machine-specific after export.
pub fn residual_absolute_paths(portable: &Value) -> Vec<String> {
    let mut found = Vec::new();
    for node in crate::tools::items(portable.get("nodes")) {
        for (key, value) in spread(node.get("config")) {
            if let Value::String(s) = value {
                if MACHINE_PATH.is_match(&s) {
                    found.push(format!("{}.{key}", json::display(node.get("id"))));
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_as_the_typescript_does() {
        assert_eq!(slugify("Deploy the App!"), "deploy-the-app");
        assert_eq!(slugify("--Hello--"), "hello");
        assert_eq!(slugify("¡¿"), "workflow");
        assert_eq!(slugify(&"a".repeat(70)), "a".repeat(60));
        assert_eq!(
            slugify(&format!("{}-b", "a".repeat(59))),
            format!("{}-", "a".repeat(59))
        );
    }

    #[test]
    fn paths_become_placeholders_and_back() {
        let workflow = json!({
            "id": "w", "name": "Ship It", "icon": "Zap", "iconColor": "", "edges": [],
            "nodes": [
                { "id": "s", "type": "script", "label": "S", "position": {"x": 0, "y": 0},
                  "config": { "cwd": "/home/me/app/sub", "projectPath": "/home/me/app",
                              "projectName": "app", "remoteHostId": "h", "secretsFrom": "c1",
                              "note": "/home/me/elsewhere" } },
                { "id": "c", "type": "callConnectorAction", "label": "C", "position": {"x": 0, "y": 0},
                  "config": { "connectionId": "c2", "event": "e" } }
            ]
        });
        let connections = [
            json!({ "id": "c1", "name": "Keys", "connectorId": "secrets" }),
            json!({ "id": "c2", "name": "Linear", "connectorId": "sdk", "filters": { "sdkConnectorId": "linear" } }),
        ];
        let portable = to_portable(&workflow, "/home/me/app/", &connections);
        assert_eq!(portable["slug"], "ship-it");
        assert!(portable.get("iconColor").is_none());
        let script = &portable["nodes"][0]["config"];
        assert_eq!(script["cwd"], "{{project.path}}/sub");
        assert_eq!(script["projectPath"], "{{project.path}}");
        assert_eq!(script["projectName"], "{{project.name}}");
        assert!(script.get("remoteHostId").is_none() && script.get("secretsFrom").is_none());
        assert_eq!(portable["nodes"][1]["config"]["connectionId"], "");
        assert_eq!(
            portable["requires"],
            json!([
                { "kind": "connection", "nodeId": "s", "connectorId": "secrets", "name": "Keys", "key": "secretsFrom" },
                { "kind": "connection", "nodeId": "c", "connectorId": "linear", "name": "Linear", "event": "e" }
            ])
        );
        assert_eq!(residual_absolute_paths(&portable), ["s.note"]);

        let back = from_portable(
            &portable,
            "app",
            &json!("other"),
            "/srv/other/",
            &connections,
        );
        assert_eq!(back["id"], "import:app:ship-it");
        assert_eq!(back["iconColor"], "#6366f1");
        assert_eq!(back["nodes"][0]["config"]["cwd"], "/srv/other/sub");
        assert_eq!(back["nodes"][0]["config"]["projectName"], "other");
        assert_eq!(back["nodes"][1]["config"]["connectionId"], "c2");
        let unresolved = unresolved_requirements(&portable, &connections);
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0]["nodeId"], "s");
    }

    #[test]
    fn a_taken_id_of_another_name_steps_aside() {
        let existing = [
            json!({ "id": "import:b:s", "name": "Other" }),
            json!({ "id": "import:b:s-2", "name": "Mine" }),
        ];
        assert_eq!(
            imported_workflow_id_for("b", "s", Some(&json!("Mine")), &existing),
            "import:b:s-2"
        );
        assert_eq!(
            imported_workflow_id_for("b", "s", Some(&json!("New")), &existing),
            "import:b:s-3"
        );
    }
}

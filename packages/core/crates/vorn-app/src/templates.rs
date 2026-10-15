//! `template-requirements`: what a template or a canvas still needs bound,
//! what each gap offers to do about it, and what a template becomes.

use serde_json::{json, Value};
use vorn_mcp::workflow::portability as port;

use crate::library::connection_connector;
use crate::runs::s;

/// A requirement and the connection this machine would bind to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Need {
    pub req: Value,
    pub bound: Option<Value>,
}

/// `RequirementAction`: the one step a requirement row offers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Fix {
    None,
    Install { id: String, name: String },
    AddConnection { id: String, name: String },
    CreateProfile(String),
}

impl Fix {
    /// `ACTION_LABEL`.
    pub fn label(&self) -> Option<String> {
        match self {
            Fix::None => None,
            Fix::Install { name, .. } => Some(format!("Install {name}")),
            Fix::AddConnection { .. } => Some("Add connection".into()),
            Fix::CreateProfile(_) => Some("Create profile".into()),
        }
    }
}

/// `ConnectorSuggestion`: a workflow a connected connector already knows how to build.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub key: String,
    pub connection_id: String,
    pub connection_name: String,
    pub event: String,
    pub name: String,
}

/// What the requirement rows look up a connector in.
#[derive(Debug, Clone, Copy)]
pub struct Listings<'a> {
    pub connectors: &'a [Value],
    pub catalog: &'a [Value],
    pub packs: &'a [Value],
}

/// `describeRequirement`.
pub fn describe(r: &Value) -> String {
    let name = s(r, "name").filter(|n| !n.is_empty());
    if s(r, "kind") == Some("httpProfile") {
        return name.map_or_else(
            || "an HTTP profile".to_owned(),
            |n| format!("an HTTP profile like \"{n}\""),
        );
    }
    let connector = s(r, "connectorId")
        .filter(|c| !c.is_empty())
        .unwrap_or("connector");
    if s(r, "key") == Some("secretsFrom") {
        let used = name
            .map(|n| format!(" (it used \"{n}\")"))
            .unwrap_or_default();
        return format!("a {connector} key for its Secrets from{used}");
    }
    name.map_or_else(
        || format!("a {connector} connection"),
        |n| format!("a {connector} connection like \"{n}\""),
    )
}

/// `connectorSuggestions`.
pub fn suggestions(connections: &[Value], connectors: &[Value]) -> Vec<Suggestion> {
    connections
        .iter()
        .flat_map(|c| {
            let id = connection_connector(c);
            let seeded = connectors
                .iter()
                .find(|k| s(k, "id") == Some(id))
                .and_then(|k| k.pointer("/manifest/defaultWorkflows"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let cid = s(c, "id").unwrap_or("").to_owned();
            let cname = s(c, "name").unwrap_or("").to_owned();
            seeded.into_iter().map(move |w| {
                let event = s(&w, "event").unwrap_or("").to_owned();
                Suggestion {
                    key: format!("{cid}:{event}"),
                    connection_id: cid.clone(),
                    connection_name: cname.clone(),
                    name: s(&w, "name").unwrap_or("").to_owned(),
                    event,
                }
            })
        })
        .collect()
}

/// `requirementsOfDefinition`: what the canvas is still missing.
pub fn of_definition(nodes: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for node in nodes {
        let config = node
            .get("config")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let Some(key) = port::bound_connection_key(node, &config) else {
            continue;
        };
        if config
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
        {
            continue;
        }
        let id = node.get("id").cloned().unwrap_or(Value::Null);
        if key == "profileConnectionId" {
            // A request with no profile field at all goes to a public URL.
            if config.contains_key(key) {
                out.push(json!({ "kind": "httpProfile", "nodeId": id, "name": "" }));
            }
            continue;
        }
        let connector = config
            .get("connectorId")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !connector.is_empty() {
            out.push(
                json!({ "kind": "connection", "nodeId": id, "connectorId": connector, "name": "" }),
            );
        }
    }
    out
}

/// `mergeRequirements`: one list of gaps from the canvas and an import.
pub fn merge(derived: Vec<Value>, imported: &[Value]) -> Vec<Value> {
    let mut rest: Vec<Value> = imported.to_vec();
    let mut out: Vec<Value> = derived
        .into_iter()
        .map(|canvas| {
            let at = rest
                .iter()
                .position(|r| r.get("nodeId") == canvas.get("nodeId"));
            let Some(file) = at.map(|i| rest.remove(i)) else {
                return canvas;
            };
            if s(&file, "kind") != Some("connection") || s(&canvas, "kind") != Some("connection") {
                return file;
            }
            let mut merged = file.clone();
            for key in ["connectorId", "name"] {
                if s(&file, key).is_none_or(str::is_empty) {
                    merged[key] = canvas.get(key).cloned().unwrap_or(json!(""));
                }
            }
            merged
        })
        .collect();
    out.extend(rest);
    out
}

/// `requirementsWithBindings`.
pub fn with_bindings(reqs: &[Value], connections: &[Value]) -> Vec<Need> {
    reqs.iter()
        .map(|r| Need {
            req: r.clone(),
            bound: if port::binds_only_by_hand(r) {
                None
            } else {
                port::resolve_requirement(r, connections)
            },
        })
        .collect()
}

/// `templateRequirements`.
pub fn of_template(template: &Value, connections: &[Value]) -> Vec<Need> {
    let reqs = template
        .pointer("/portable/requires")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    with_bindings(&reqs, connections)
}

/// `requirementAction`, over the listings `buildConnectorListings` would make.
pub fn fix(need: &Need, l: Listings<'_>) -> Fix {
    let r = &need.req;
    if need.bound.is_some() || port::binds_only_by_hand(r) {
        return Fix::None;
    }
    if s(r, "kind") == Some("httpProfile") {
        return Fix::CreateProfile(s(r, "name").unwrap_or("").to_owned());
    }
    let id = s(r, "connectorId").unwrap_or("");
    if id.is_empty() {
        return Fix::None;
    }
    let named = |v: &Value| s(v, "name").unwrap_or(id).to_owned();
    let pack = l.packs.iter().find(|p| s(p, "id") == Some(id));
    if let Some(b) = l.connectors.iter().find(|c| s(c, "id") == Some(id)) {
        if b.get("addable") != Some(&Value::Bool(false)) {
            return Fix::AddConnection {
                id: id.into(),
                name: named(b),
            };
        }
    }
    if let Some(entry) = l.catalog.iter().find(|c| s(c, "id") == Some(id)) {
        let released = crate::plugins::installable(entry);
        if pack.is_none() && !released {
            return Fix::None;
        }
        let kind = pack.and_then(|p| s(p, "kind")).or_else(|| s(entry, "kind"));
        let legacy = s(entry, "packageName").is_some_and(|p| !p.is_empty());
        let can_add = kind != Some("extension") && (pack.is_some() || legacy);
        return if can_add {
            Fix::AddConnection {
                id: id.into(),
                name: named(entry),
            }
        } else {
            Fix::Install {
                id: id.into(),
                name: named(entry),
            }
        };
    }
    match pack {
        Some(p) if s(p, "kind") == Some("extension") => Fix::Install {
            id: id.into(),
            name: named(p),
        },
        Some(p) => Fix::AddConnection {
            id: id.into(),
            name: named(p),
        },
        None => Fix::None,
    }
}

/// `templateSeed`: the definition a template starts the canvas with, and
/// what it left unbound.
pub fn seed(template: &Value, project: (&str, &str), connections: &[Value]) -> (Value, Vec<Value>) {
    let portable = template.get("portable").cloned().unwrap_or(Value::Null);
    let mut def = port::from_portable(
        &portable,
        "template",
        &json!(project.0),
        project.1,
        connections,
    );
    for node in def["nodes"].as_array_mut().into_iter().flatten() {
        if node.pointer("/config/triggerType").and_then(Value::as_str) == Some("webhook") {
            // This machine's own token, never one the catalog carried.
            node["config"]["token"] = json!(crate::graph::new_id());
        }
    }
    (def, port::unresolved_requirements(&portable, connections))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn need(r: Value) -> Need {
        Need {
            req: r,
            bound: None,
        }
    }

    #[test]
    fn a_requirement_offers_the_fix_its_connector_allows() {
        let builtins = [json!({ "id": "github", "name": "GitHub" })];
        let catalog = [
            json!({ "id": "jira", "name": "Jira", "git": "https://x.example/c.git" }),
            json!({ "id": "soon", "name": "Soon" }),
        ];
        let packs = [json!({ "id": "jira", "version": "1.0.0" })];
        let l = Listings {
            connectors: &builtins,
            catalog: &catalog,
            packs: &[],
        };
        let conn = |c: &str| {
            need(json!({ "kind": "connection", "nodeId": "n", "connectorId": c, "name": "" }))
        };
        assert_eq!(
            fix(&conn("github"), l).label().as_deref(),
            Some("Add connection")
        );
        assert_eq!(
            fix(&conn("jira"), l).label().as_deref(),
            Some("Install Jira")
        );
        assert_eq!(fix(&conn("soon"), l), Fix::None);
        assert_eq!(fix(&conn(""), l), Fix::None);
        let installed = Listings { packs: &packs, ..l };
        assert_eq!(
            fix(&conn("jira"), installed).label().as_deref(),
            Some("Add connection")
        );
        let http = need(json!({ "kind": "httpProfile", "nodeId": "n", "name": "api" }));
        assert_eq!(fix(&http, l), Fix::CreateProfile("api".into()));
        let key = need(
            json!({ "kind": "connection", "nodeId": "n", "connectorId": "github", "name": "", "key": "secretsFrom" }),
        );
        assert_eq!(fix(&key, l), Fix::None);
    }

    #[test]
    fn the_canvas_names_its_unbound_steps_and_an_import_fills_in_names() {
        let nodes = [
            json!({ "id": "h", "type": "httpRequest", "config": { "profileConnectionId": "" } }),
            json!({ "id": "p", "type": "httpRequest", "config": {} }),
            json!({ "id": "a", "type": "callConnectorAction", "config": { "connectorId": "github", "connectionId": "" } }),
            json!({ "id": "b", "type": "callConnectorAction", "config": { "connectorId": "github", "connectionId": "c1" } }),
        ];
        let derived = of_definition(&nodes);
        assert_eq!(derived.len(), 2);
        let imported = [
            json!({ "kind": "connection", "nodeId": "a", "connectorId": "", "name": "work" }),
            json!({ "kind": "connection", "nodeId": "z", "connectorId": "x", "name": "" }),
        ];
        let merged = merge(derived, &imported);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[1]["connectorId"], "github");
        assert_eq!(merged[1]["name"], "work");
        assert_eq!(describe(&merged[1]), "a github connection like \"work\"");
    }

    #[test]
    fn a_connection_suggests_its_connectors_default_workflows() {
        let conns = [json!({ "id": "c1", "name": "Work", "connectorId": "github" })];
        let connectors = [
            json!({ "id": "github", "manifest": { "defaultWorkflows": [{ "event": "pr", "name": "Review PRs" }] } }),
        ];
        let got = suggestions(&conns, &connectors);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].key, "c1:pr");
        assert_eq!(got[0].name, "Review PRs");
    }

    #[test]
    fn a_webhook_template_gets_its_own_token() {
        let t = json!({ "portable": { "version": 1, "slug": "w", "name": "W", "nodes": [
            { "id": "t", "type": "trigger", "config": { "triggerType": "webhook", "token": "shared" } }
        ], "edges": [] } });
        let (def, unresolved) = seed(&t, ("p", "/p"), &[]);
        assert_ne!(def["nodes"][0]["config"]["token"], "shared");
        assert!(unresolved.is_empty());
    }
}

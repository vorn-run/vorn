//! How an extension's manifest fills the workflow registry: each
//! `[[contributes.nodes]]` becomes a node type and each
//! `[[contributes.triggers]]` a trigger type, under the extension's id.
//! vornd declares every extension this way, those Vorn ships and those
//! installed from git alike.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Map, Value};
use vorn_extensions::manifest::workflows::TriggerTiming;
use vorn_extensions::manifest::Manifest;
use vorn_work::trigger::Timing;
use vorn_workflow::{Host, NodeType, Registry, Runner, Step, TriggerType};

/// What a node type's doc holds when its extension ships none: what the
/// manifest says of it, and its config's schema when the package has one.
fn doc_from(node: &vorn_extensions::manifest::workflows::NodeDecl, schema: Option<Value>) -> Value {
    let outputs: Vec<Value> = node
        .outputs
        .iter()
        .map(|o| json!({ "field": o.field, "description": o.description }))
        .collect();
    json!({
        "summary": node.base.description.clone().unwrap_or_else(|| node.base.title.clone()),
        "configSchema": schema.unwrap_or_else(|| json!({ "type": "object" })),
        "outputs": outputs,
        "rules": node.rules,
    })
}

/// Declares what `manifest` contributes to workflows, available when `on`.
/// A node type's doc is `docs[id]` when given, else built from the manifest
/// and `schema(path)`; it runs by `steps[id]` in vornd when given, else by a
/// call to the extension. Returns the ids another extension already holds,
/// which stay theirs.
pub fn declare<H: Host>(
    registry: &Registry<H>,
    manifest: &Manifest,
    docs: &Map<String, Value>,
    steps: &HashMap<&str, Arc<dyn Step<H>>>,
    schema: &dyn Fn(&str) -> Option<Value>,
    on: bool,
) -> Vec<String> {
    let mut taken = Vec::new();
    let Some(c) = &manifest.contributes else {
        return taken;
    };
    for node in c.nodes() {
        let id = node.base.id.as_str();
        let mut doc = match docs.get(id) {
            Some(d) => d.clone(),
            None => doc_from(node, node.config.as_deref().and_then(schema)),
        };
        if let Value::Object(d) = &mut doc {
            d.insert("title".into(), json!(node.base.title));
            if let Some(icon) = &node.icon {
                d.insert("icon".into(), json!(icon));
            }
            if let Some(category) = &node.category {
                d.insert("category".into(), json!(category));
            }
            d.insert("extension".into(), json!(manifest.id));
        }
        let runner = on.then(|| match steps.get(id) {
            Some(step) => Runner::Local(Arc::clone(step)),
            None => Runner::Call,
        });
        let node = NodeType {
            id: id.to_owned(),
            provider: manifest.id.clone(),
            provider_name: manifest.name.clone(),
            doc,
        };
        if !registry.add_node(node, runner) {
            taken.push(id.to_owned());
        }
    }
    for trigger in c.triggers() {
        let mut doc = json!({ "title": trigger.base.title, "extension": manifest.id });
        if let Some(icon) = &trigger.icon {
            doc["icon"] = json!(icon);
        }
        if let Some(d) = &trigger.base.description {
            doc["summary"] = json!(d);
        }
        if let Some(schema) = trigger.config.as_deref().and_then(schema) {
            doc["configSchema"] = schema;
        }
        let declared = TriggerType {
            id: trigger.base.id.clone(),
            provider: manifest.id.clone(),
            provider_name: manifest.name.clone(),
            timing: trigger.timing.map(|t| match t {
                TriggerTiming::Cron => Timing::Cron,
                TriggerTiming::Once => Timing::Once,
                TriggerTiming::Webhook => Timing::Webhook,
            }),
            fans_out: trigger.poll,
            doc,
        };
        if !registry.add_trigger(declared, on) {
            taken.push(trigger.base.id.clone());
        }
    }
    taken
}

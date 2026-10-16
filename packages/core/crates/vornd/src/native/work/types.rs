//! The node types and trigger types the engine's registry holds, as vornd
//! fills it: the built-ins' (Workflow steps and triggers, Tasks and
//! Connectors) and those of every installed extension that declares some,
//! declared while each is on and unavailable while it is off. A step of an
//! extension's type is run by a call to it: the built-ins answer in vornd,
//! a program over its pipe.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use serde_json::{Map, Value};
use tracing::warn;
use vorn_extensions::api::types::CallCtx;
use vorn_extensions::manifest::{file, Manifest};
use vorn_workflow::{Registry, Step};

use super::host::VorndHost;
use crate::native::extensions::builtin::switched_on;
use crate::native::Native;

static WORKFLOWS: LazyLock<Manifest> = LazyLock::new(vorn_workflows::declare::manifest);

/// The Workflows built-in's manifest, which its calls are held to.
pub fn workflows_manifest() -> Manifest {
    WORKFLOWS.clone()
}

/// One extension that may declare workflow types.
struct Provider {
    manifest: Manifest,
    docs: Map<String, Value>,
    /// Where its package is, for its config schemas; none for a built-in.
    dir: Option<PathBuf>,
    on: bool,
}

/// Whether built-in `id` is on: as the extension host's registry says, or
/// before it starts, as the switches file does.
fn builtin_on(native: &Native, id: &str) -> bool {
    if let Some(ext) = native.extensions() {
        return ext.builtins().is_on(id);
    }
    native
        .database()
        .and_then(std::path::Path::parent)
        .is_none_or(|dir| switched_on(&dir.join("builtins.json"), id, true))
}

fn docs(text: &str) -> Map<String, Value> {
    serde_json::from_str(text).unwrap_or_default()
}

fn parsed(text: &str) -> Option<Manifest> {
    file::parse_builtin(text)
        .map_err(|err| warn!(%err, "a built-in's manifest does not read"))
        .ok()
}

/// Every extension that declares workflow types, the built-ins first.
fn providers(native: &Native) -> Vec<Provider> {
    let mut out = Vec::new();
    for (text, node_docs) in [
        (vorn_workflows::MANIFEST, vorn_workflows::NODE_DOCS),
        (vorn_tasks::MANIFEST, vorn_tasks::NODE_DOCS),
        (
            vorn_connectors::builtin::MANIFEST,
            vorn_connectors::builtin::NODE_DOCS,
        ),
    ] {
        let Some(manifest) = parsed(text) else {
            continue;
        };
        let on = builtin_on(native, &manifest.id);
        out.push(Provider {
            manifest,
            docs: docs(node_docs),
            dir: None,
            on,
        });
    }
    if let Some(ext) = native.extensions() {
        for (manifest, dir) in ext.workflow_providers() {
            out.push(Provider {
                manifest,
                docs: Map::new(),
                dir: Some(dir),
                on: true,
            });
        }
    }
    out
}

/// Declares every provider that is on into `registry`, and marks the types
/// of those that are off unavailable.
pub fn sync(native: &Native, registry: &Registry<VorndHost>) {
    let steps: HashMap<&str, Arc<dyn Step<VorndHost>>> =
        vorn_workflows::steps().into_iter().collect();
    let none = HashMap::new();
    for p in providers(native) {
        if !p.on {
            registry.switch_off(&p.manifest.id);
            continue;
        }
        let local = if p.manifest.id == vorn_workflows::ID {
            &steps
        } else {
            &none
        };
        let schema = |path: &str| -> Option<Value> {
            let dir = p.dir.as_ref()?;
            let text = std::fs::read_to_string(dir.join(path)).ok()?;
            serde_json::from_str(&text).ok()
        };
        vorn_workflows::declare::declare(registry, &p.manifest, &p.docs, local, &schema);
    }
    registry.set_fallback(Some(vorn_workflows::FALLBACK));
}

/// Runs a step of `node_type` by a call to `provider`.
pub async fn run_node(
    native: &Arc<Native>,
    provider: &str,
    node_type: &str,
    request: Value,
) -> Result<Value, String> {
    if provider == vorn_tasks::ID {
        let tasks = native.tasks().cloned().ok_or("this vornd keeps no tasks")?;
        return tasks.run_node(node_type, &request).await;
    }
    if provider == vorn_connectors::builtin::ID {
        let connectors = native
            .connectors()
            .cloned()
            .ok_or("this vornd keeps no connections")?;
        return connectors.run_node(node_type, &request).await;
    }
    let ext = native
        .extensions()
        .cloned()
        .ok_or("vornd is not hosting extensions")?;
    let ctx = CallCtx {
        caller: vorn_workflows::ID.to_owned(),
        project: None,
    };
    let action = format!("workflow.node.{node_type}");
    match ext
        .builtins()
        .call(provider, &action, request.clone())
        .await
    {
        Some(answered) => answered.map_err(|e| e.message),
        None => ext
            .router()
            .call(provider, &action, request, ctx)
            .await
            .map_err(|e| e.message),
    }
}

/// `workflow:nodeTypes`: what the editor's step library lists, the node
/// types and trigger types whose extensions are on, each with its doc.
pub fn listed(registry: &Registry<VorndHost>) -> Value {
    let nodes: Vec<Value> = registry
        .nodes()
        .into_iter()
        .map(|n| serde_json::json!({ "id": n.id, "extension": n.provider, "extensionName": n.provider_name, "doc": n.doc }))
        .collect();
    let triggers: Vec<Value> = registry
        .triggers()
        .into_iter()
        .map(|t| serde_json::json!({ "id": t.id, "extension": t.provider, "extensionName": t.provider_name, "doc": t.doc, "poll": t.fans_out }))
        .collect();
    serde_json::json!({ "nodes": nodes, "triggers": triggers })
}

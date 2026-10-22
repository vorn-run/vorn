//! The node types and trigger types the engine's registry holds, as vornd
//! fills it: Connectors' (built in) and those of every installed extension
//! that declares some (Workflows and Tasks among them), declared while each
//! is on and unavailable while it is off or gone. A step of an extension's
//! type is run by a call to it: Connectors answers in vornd, an install
//! through the router.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Map, Value};
use tracing::warn;
use vorn_extensions::api::types::CallCtx;
use vorn_extensions::manifest::{file, Manifest};
use vorn_workflow::{Registry, Step};

use super::host::VorndHost;
use crate::native::extensions::builtin::switched_on;
use crate::native::Native;

/// The installed Workflows extension's manifest, which its calls are held to.
pub fn workflows_manifest(native: &Native) -> Result<Manifest, String> {
    native
        .extensions()
        .and_then(|ext| ext.router().program(vorn_workflows::ID))
        .map(|pack| pack.manifest.clone())
        .ok_or_else(|| crate::native::extensions::hosted::not_installed("Workflows"))
}

/// One extension that may declare workflow types.
struct Provider {
    manifest: Manifest,
    docs: Map<String, Value>,
    /// Where its package is, for its config schemas; none for a built-in.
    dir: Option<PathBuf>,
    on: bool,
}

/// Whether Connectors is on: as the extension host's registry says, or
/// before it starts, as the switches file does.
fn connectors_on(native: &Native) -> bool {
    let id = vorn_connectors::builtin::ID;
    if let Some(ext) = native.extensions() {
        return ext.builtins().is_on(id);
    }
    native
        .database()
        .and_then(std::path::Path::parent)
        .is_none_or(|dir| switched_on(&dir.join("builtins.json"), id, true))
}

/// Every extension that declares workflow types: Connectors, then
/// Workflows and Tasks, so the types Vorn ships stay theirs, then the rest.
fn providers(native: &Native) -> Vec<Provider> {
    let mut out = Vec::new();
    match file::parse_builtin(vorn_connectors::builtin::MANIFEST) {
        Ok(manifest) => out.push(Provider {
            manifest,
            docs: serde_json::from_str(vorn_connectors::builtin::NODE_DOCS).unwrap_or_default(),
            dir: None,
            on: connectors_on(native),
        }),
        Err(err) => warn!(%err, "Connectors' manifest does not read"),
    }
    #[cfg(test)]
    if native.extensions().is_none() {
        out.extend(shipped_for_tests());
    }
    if let Some(ext) = native.extensions() {
        let mut installed = ext.workflow_providers();
        let rank = |id: &str| {
            [vorn_workflows::ID, vorn_workflows::TASKS_ID]
                .iter()
                .position(|first| *first == id)
                .unwrap_or(usize::MAX)
        };
        installed.sort_by_key(|(m, _)| rank(&m.id));
        for (manifest, dir) in installed {
            out.push(Provider {
                docs: vorn_workflows::docs_of(&manifest.id),
                manifest,
                dir: Some(dir),
                on: true,
            });
        }
    }
    out
}

/// Workflows and Tasks as if installed, for the engine's tests here, which run without an extension host.
#[cfg(test)]
fn shipped_for_tests() -> Vec<Provider> {
    [
        include_str!("../../../../workflows/tests/fixtures/workflows.toml"),
        include_str!("../../../../workflows/tests/fixtures/tasks.toml"),
    ]
    .into_iter()
    .map(|text| {
        let manifest = file::parse_builtin(text).expect("a fixture manifest reads");
        Provider {
            docs: vorn_workflows::docs_of(&manifest.id),
            manifest,
            dir: None,
            on: true,
        }
    })
    .collect()
}

/// Declares every provider into `registry`, the built-ins first so their
/// types stay theirs, and marks unavailable the types of those that are off
/// or gone.
pub fn sync(native: &Native, registry: &Registry<VorndHost>) {
    let steps: HashMap<&str, Arc<dyn Step<VorndHost>>> =
        vorn_workflows::steps().into_iter().collect();
    let none = HashMap::new();
    let listed = providers(native);
    for p in &listed {
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
        let taken =
            vorn_workflows::declare::declare(registry, &p.manifest, &p.docs, local, &schema, p.on);
        if !taken.is_empty() {
            warn!(extension = %p.manifest.id, ?taken, "workflow types another extension declares were left with it");
        }
    }
    for gone in registry
        .providers()
        .into_keys()
        .filter(|id| !listed.iter().any(|p| p.manifest.id == *id))
    {
        registry.switch_off(&gone);
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

/// The workflows installed extensions ship (`[[contributes.workflows]]`):
/// each one's extension, id and file text. The built-ins' are seeded with
/// the database.
pub fn contributed_workflows(native: &Native) -> Vec<(String, String, String)> {
    let Some(ext) = native.extensions() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (manifest, dir) in ext.workflow_providers() {
        let Some(c) = &manifest.contributes else {
            continue;
        };
        for w in c.workflows() {
            match std::fs::read_to_string(dir.join(&w.file)) {
                Ok(text) => out.push((manifest.id.clone(), w.base.id.clone(), text)),
                Err(err) => {
                    warn!(%err, extension = %manifest.id, file = %w.file, "a shipped workflow cannot be read")
                }
            }
        }
    }
    out
}

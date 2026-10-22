//! The node types and trigger types workflows are built from, as
//! extensions fill them in. The engine walks four kinds itself (a run's
//! start, a branch, a gate and a loop); every other step is a node type
//! some extension declares and runs, and every trigger but none is a
//! trigger type some extension declares and fires.
//!
//! A type stays known once declared: when its extension is switched off or
//! removed it is unavailable rather than forgotten, so a step of it fails
//! saying which extension it needs instead of running as something else.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde_json::{Map, Value};
use vorn_work::trigger::{Timing, Timings};

use crate::host::Host;
use crate::step::Step;

/// How a node type's steps are run.
pub enum Runner<H: Host> {
    /// In vornd, by the extension that declares it, with the run at hand.
    Local(Arc<dyn Step<H>>),
    /// By a call to the extension that declares it ([`Host::run_node`]):
    /// the step's config and context go out, the step's state comes back.
    Call,
}

impl<H: Host> Clone for Runner<H> {
    fn clone(&self) -> Self {
        match self {
            Runner::Local(step) => Runner::Local(Arc::clone(step)),
            Runner::Call => Runner::Call,
        }
    }
}

impl<H: Host> std::fmt::Debug for Runner<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Runner::Local(_) => "Local",
            Runner::Call => "Call",
        })
    }
}

/// A node type as its extension declares it.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeType {
    /// The step's `type` in a workflow, as `launchAgent`.
    pub id: String,
    /// The extension that declares and runs it.
    pub provider: String,
    /// That extension's name, for a step that cannot run while it is off.
    pub provider_name: String,
    /// What the editor and `describe_workflow_nodes` read: `summary`,
    /// `configSchema`, `outputs` and `rules`, and the editor's `title`,
    /// `icon` and `category`.
    pub doc: Value,
}

/// A trigger type as its extension declares it.
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerType {
    /// The trigger's `triggerType`, as `taskCreated`.
    pub id: String,
    pub provider: String,
    pub provider_name: String,
    /// When the core fires it itself; `None` when its extension fires it.
    pub timing: Option<Timing>,
    /// Its extension fans a fire out into items (a connector's poll): a
    /// person's Run asks the extension rather than starting a run.
    pub fans_out: bool,
    pub doc: Value,
}

struct Entry<T, R> {
    declared: T,
    /// `None` while its extension is off.
    runner: Option<R>,
}

struct Inner<H: Host> {
    /// Shared, so resolving a step clones a pointer and not the type's doc.
    nodes: Vec<Entry<Arc<NodeType>, Runner<H>>>,
    triggers: Vec<Entry<TriggerType, ()>>,
    /// The node type a step of a type no extension declares runs as.
    fallback: Option<String>,
}

impl<H: Host> Inner<H> {
    /// The entry a step of `node_type` runs by: its own, else the fallback's.
    fn entry(&self, node_type: &str) -> Option<&Entry<Arc<NodeType>, Runner<H>>> {
        let find = |id: &str| self.nodes.iter().find(|e| e.declared.id == id);
        find(node_type).or_else(|| self.fallback.as_deref().and_then(find))
    }
}

/// Every node type and trigger type declared so far.
pub struct Registry<H: Host> {
    inner: RwLock<Inner<H>>,
}

impl<H: Host> Default for Registry<H> {
    fn default() -> Self {
        Registry {
            inner: RwLock::new(Inner {
                nodes: Vec::new(),
                triggers: Vec::new(),
                fallback: None,
            }),
        }
    }
}

impl<H: Host> std::fmt::Debug for Registry<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.read();
        f.debug_struct("Registry")
            .field("nodes", &inner.nodes.len())
            .field("triggers", &inner.triggers.len())
            .finish()
    }
}

/// How a step of some type is to run.
pub enum Resolved<H: Host> {
    /// By this runner, of this extension.
    Run(Arc<NodeType>, Runner<H>),
    /// Its extension is off.
    Off(Arc<NodeType>),
    /// No extension declares it, and nothing stands in.
    Unknown,
}

impl<H: Host> Registry<H> {
    fn read(&self) -> RwLockReadGuard<'_, Inner<H>> {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner<H>> {
        self.inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Declares `node`, run by `runner` or unavailable while `None`; a type
    /// keeps its place, and stays with the extension that declared it first.
    pub fn add_node(&self, node: NodeType, runner: Option<Runner<H>>) -> bool {
        let mut inner = self.write();
        match inner.nodes.iter_mut().find(|e| e.declared.id == node.id) {
            Some(e) if e.declared.provider != node.provider => return false,
            Some(e) => {
                e.declared = Arc::new(node);
                e.runner = runner;
            }
            None => inner.nodes.push(Entry {
                declared: Arc::new(node),
                runner,
            }),
        }
        true
    }

    /// Declares `trigger`, available when `on`; as [`Registry::add_node`].
    pub fn add_trigger(&self, trigger: TriggerType, on: bool) -> bool {
        let mut inner = self.write();
        let runner = on.then_some(());
        match inner
            .triggers
            .iter_mut()
            .find(|e| e.declared.id == trigger.id)
        {
            Some(e) if e.declared.provider != trigger.provider => return false,
            Some(e) => {
                e.declared = trigger;
                e.runner = runner;
            }
            None => inner.triggers.push(Entry {
                declared: trigger,
                runner,
            }),
        }
        true
    }

    /// The node type a step of a type nobody declares runs as, as steps of
    /// unknown types always ran as agents.
    pub fn set_fallback(&self, node_type: Option<&str>) {
        self.write().fallback = node_type.map(str::to_owned);
    }

    /// Everything `provider` declared is unavailable until it declares it again.
    pub fn switch_off(&self, provider: &str) {
        let mut inner = self.write();
        for e in inner
            .nodes
            .iter_mut()
            .filter(|e| e.declared.provider == provider)
        {
            e.runner = None;
        }
        for e in inner
            .triggers
            .iter_mut()
            .filter(|e| e.declared.provider == provider)
        {
            e.runner = None;
        }
    }

    /// The extension that declares `node_type` itself, on or off; `None`
    /// for a type nobody declares, which runs as the fallback.
    pub fn provider(&self, node_type: &str) -> Option<String> {
        let inner = self.read();
        let entry = inner.nodes.iter().find(|e| e.declared.id == node_type);
        entry.map(|e| e.declared.provider.clone())
    }

    /// The declared node type a step of `node_type` runs as: its own, else
    /// the fallback; `None` when neither is declared.
    pub fn runs_as(&self, node_type: &str) -> Option<String> {
        self.read().entry(node_type).map(|e| e.declared.id.clone())
    }

    /// How a step of `node_type` runs.
    pub fn resolve(&self, node_type: &str) -> Resolved<H> {
        let inner = self.read();
        match inner.entry(node_type) {
            Some(Entry {
                declared,
                runner: Some(runner),
            }) => Resolved::Run(Arc::clone(declared), runner.clone()),
            Some(Entry {
                declared,
                runner: None,
            }) => Resolved::Off(Arc::clone(declared)),
            None => Resolved::Unknown,
        }
    }

    /// The node types whose extensions are on, in the order declared.
    pub fn nodes(&self) -> Vec<NodeType> {
        self.read()
            .nodes
            .iter()
            .filter(|e| e.runner.is_some())
            .map(|e| NodeType::clone(&e.declared))
            .collect()
    }

    /// The trigger types whose extensions are on, in the order declared.
    pub fn triggers(&self) -> Vec<TriggerType> {
        self.read()
            .triggers
            .iter()
            .filter(|e| e.runner.is_some())
            .map(|e| e.declared.clone())
            .collect()
    }

    /// Trigger type `id`, while its extension is on.
    pub fn trigger(&self, id: &str) -> Option<TriggerType> {
        self.read()
            .triggers
            .iter()
            .find(|e| e.declared.id == id && e.runner.is_some())
            .map(|e| e.declared.clone())
    }

    /// How each trigger type the core fires itself fires, while its extension is on.
    pub fn timings(&self) -> Timings {
        self.read()
            .triggers
            .iter()
            .filter(|e| e.runner.is_some())
            .filter_map(|e| Some((e.declared.id.clone(), e.declared.timing?)))
            .collect()
    }

    /// Whether a run of a workflow with this trigger type is fanned out by
    /// its extension rather than started.
    pub fn fans_out(&self, trigger_type: Option<&str>) -> bool {
        trigger_type
            .and_then(|t| self.trigger(t))
            .is_some_and(|t| t.fans_out)
    }

    /// The node types' docs as `describe_workflow_nodes` lists them, by id
    /// in the order declared.
    pub fn docs(&self) -> Map<String, Value> {
        let mut out = Map::new();
        for e in self.read().nodes.iter().filter(|e| e.runner.is_some()) {
            out.insert(e.declared.id.clone(), e.declared.doc.clone());
        }
        out
    }

    /// The providers declaring something, by id: what Settings names.
    pub fn providers(&self) -> BTreeMap<String, String> {
        let inner = self.read();
        inner
            .nodes
            .iter()
            .map(|e| &e.declared)
            .map(|d| (d.provider.clone(), d.provider_name.clone()))
            .chain(inner.triggers.iter().map(|e| {
                (
                    e.declared.provider.clone(),
                    e.declared.provider_name.clone(),
                )
            }))
            .collect()
    }
}

/// The engine's part of the workflow reference: the format, its limits, the
/// four kinds it walks itself and every type's usual place in the list.
pub static REFERENCE: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    serde_json::from_str(include_str!("../generated/reference.json"))
        .expect("generated/reference.json is JSON")
});

/// The keys of a node type's doc the reference lists; the rest are the editor's.
const DOC_KEYS: [&str; 4] = ["summary", "configSchema", "outputs", "rules"];

impl<H: Host> Registry<H> {
    /// The reference `describe_workflow_nodes` answers with: the engine's
    /// own kinds and every node type whose extension is on, Vorn's in their
    /// usual order and then the rest as declared, beside the format.
    pub fn describe(&self) -> Value {
        let declared = self.docs();
        let core = REFERENCE.get("nodeTypes").and_then(Value::as_object);
        let order: Vec<&str> = REFERENCE
            .get("order")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let listed = |doc: &Value| -> Value {
            let mut out = Map::new();
            for key in DOC_KEYS {
                if let Some(v) = doc.get(key) {
                    out.insert(key.to_owned(), v.clone());
                }
            }
            Value::Object(out)
        };
        let mut types = Map::new();
        for id in &order {
            if let Some(doc) = core.and_then(|c| c.get(*id)) {
                types.insert((*id).to_owned(), doc.clone());
            } else if let Some(doc) = declared.get(*id) {
                types.insert((*id).to_owned(), listed(doc));
            }
        }
        for (id, doc) in &declared {
            if !types.contains_key(id) {
                types.insert(id.clone(), listed(doc));
            }
        }
        let mut out = Map::new();
        out.insert("nodeTypes".into(), Value::Object(types));
        for (k, v) in REFERENCE.as_object().into_iter().flatten() {
            if k != "order" && k != "nodeTypes" {
                out.insert(k.clone(), v.clone());
            }
        }
        Value::Object(out)
    }
}

/// Whether a trigger's config takes a fire with these fields: each field the
/// config sets (to anything but an empty string) must equal the fire's; a
/// field it leaves unset takes any.
pub fn matches(config: &Value, fields: &Map<String, Value>) -> bool {
    fields.iter().all(|(key, value)| {
        match config
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            None => true,
            Some(wanted) => value.as_str() == Some(wanted),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_set_field_must_equal_the_fires_and_an_unset_one_takes_any() {
        let fire = json!({ "projectFilter": "app", "toStatus": "done" });
        let fields = fire.as_object().unwrap();
        assert!(matches(&json!({}), fields));
        assert!(matches(&json!({ "projectFilter": "" }), fields));
        assert!(matches(&json!({ "projectFilter": "app" }), fields));
        assert!(!matches(&json!({ "projectFilter": "other" }), fields));
        assert!(!matches(&json!({ "toStatus": "todo" }), fields));
        // A fire without the field matches only a config that leaves it unset.
        let none = json!({ "projectFilter": null });
        assert!(!matches(
            &json!({ "projectFilter": "app" }),
            none.as_object().unwrap()
        ));
        assert!(matches(&json!({ "fromStatus": "x" }), fields));
    }
}

//! Tasks in vornd: the component Vorn ships (`packages/core/extensions/tasks`)
//! as an entry of the built-ins' registry, and the core's routes to it. The
//! clients' `task:*` writes, the other built-ins' `task.*` calls, its MCP
//! tools and its workflow step all reach the component through the router,
//! under the same gate and lifecycle as an installed component; while its
//! switch is off each is refused, and the tasks stay where they are, still
//! listed by the core.
//!
//! What a component cannot know, vornd answers: where a task's image is on
//! this machine (`task:imageGetPath` answers the file's name, which becomes
//! its path here).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::{json, Value};
use tracing::{info, warn};
use vorn_extensions::api::types::{CallCtx, Event};
use vorn_extensions::api::ExtError;
use vorn_extensions::component::Bundled;
use vorn_extensions::events::AGENT_CONVERSATION;
use vorn_extensions::manifest::{file, Manifest};

use super::{Api, Builtin};
use crate::native::{Answer, Native};

/// The id Tasks goes by.
pub const ID: &str = "vorn.tasks";

/// Its manifest, which it is held to.
pub const MANIFEST: &str = include_str!("../../../../../../extensions/tasks/vorn-extension.toml");

/// What `describe_workflow_nodes` tells of the node types it declares, by id.
pub const NODE_DOCS: &str = include_str!("../../../../../../extensions/tasks/generated/nodes.json");

/// Its MCP tools, as `tools/list` lists them.
const TOOLS: &str = include_str!("../../../../../../extensions/tasks/generated/tools.json");

/// The component, built for wasm32-wasip2 by vornd's build script.
pub(crate) const WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tasks.wasm"));

/// What a call to a task method is told while Tasks is off.
pub const OFF: &str = "Tasks is off. Turn it on in Settings → Extensions.";

/// Every `task:*` call it answers; reading and listing are the core's.
pub const METHODS: &[&str] = &[
    "task:setStatus",
    "task:create",
    "task:update",
    "task:patch",
    "task:delete",
    "task:reorder",
    "task:archive",
    "task:imageSave",
    "task:imageDelete",
    "task:imageGetPath",
    "task:imageCleanup",
    "task:imageUpload",
];

/// Where Tasks' files are kept, beside `vorn.db`: where the board has always kept its images.
pub const IMAGES: &str = "task-images";

/// Its manifest, read.
pub fn manifest() -> Manifest {
    file::parse_builtin(MANIFEST).expect("the Tasks manifest is checked by its tests")
}

/// Tasks, and whether its switch is on.
pub struct TasksBuiltin {
    native: Weak<Native>,
    manifest: Arc<Manifest>,
    on: AtomicBool,
}

impl std::fmt::Debug for TasksBuiltin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TasksBuiltin")
            .field("on", &self.on)
            .finish_non_exhaustive()
    }
}

impl TasksBuiltin {
    /// Tasks on `native`, `on` as its switch was left.
    pub fn new(native: &Arc<Native>, on: bool) -> Arc<TasksBuiltin> {
        Arc::new(TasksBuiltin {
            native: Arc::downgrade(native),
            manifest: Arc::new(manifest()),
            on: AtomicBool::new(on),
        })
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    fn set_on(&self, on: bool) {
        if self.on.swap(on, Ordering::AcqRel) != on {
            if on {
                info!("[tasks] switched on");
            } else {
                info!("[tasks] switched off; the tasks are kept");
            }
        }
    }

    fn bundled(&self) -> Bundled {
        Bundled {
            manifest: Arc::clone(&self.manifest),
            wasm: WASM,
        }
    }

    /// Calls `method` of the component, starting it for the first call.
    async fn call(&self, method: &str, args: Value, caller: &str) -> Result<Value, ExtError> {
        let native = self
            .native
            .upgrade()
            .ok_or_else(|| ExtError::new("stopping", "vornd is stopping"))?;
        let ext = native
            .extensions()
            .cloned()
            .ok_or_else(|| ExtError::new("unsupported", "vornd is not hosting extensions"))?;
        let ctx = CallCtx {
            caller: caller.to_owned(),
            project: None,
        };
        ext.router()
            .call_bundled(&self.bundled(), method, args, ctx)
            .await
    }

    /// Where Tasks' file `name` is on this machine.
    fn path_of(&self, name: &str) -> Result<String, String> {
        let native = self.native.upgrade().ok_or("vornd is stopping")?;
        let ext = native
            .extensions()
            .ok_or("vornd is not hosting extensions")?;
        let path = ext
            .router()
            .blob_path(ID, name)
            .ok_or("vornd has no database")?
            .map_err(|e| e.message)?;
        Ok(path.to_string_lossy().into_owned())
    }

    /// Answers one of its `task:*` calls; while it is off, none.
    pub async fn answer(&self, method: &str, params: Value) -> Answer {
        if !self.is_on() {
            return Answer::Error(OFF.to_owned());
        }
        if !METHODS.contains(&method) {
            return Answer::Unanswered;
        }
        let answered = match self.call(method, params, "client").await {
            Ok(v) => v,
            Err(e) => return Answer::Error(e.message),
        };
        match method {
            "task:imageGetPath" => {
                let name = answered.get("file").and_then(Value::as_str).unwrap_or("");
                match self.path_of(name) {
                    Ok(path) => Answer::Result(json!(path)),
                    Err(e) => Answer::Error(e),
                }
            }
            "task:imageDelete" | "task:imageCleanup" if answered.is_null() => Answer::Void,
            _ => Answer::Result(answered),
        }
    }

    /// A workflow step of a node type Tasks declares; refused while off.
    pub async fn run_node(&self, node_type: &str, request: &Value) -> Result<Value, String> {
        if !self.is_on() {
            return Err(OFF.to_owned());
        }
        self.call(
            &format!("workflow.node.{node_type}"),
            request.clone(),
            "workflow",
        )
        .await
        .map_err(|e| e.message)
    }

    /// `task.importItems`, `task.sourceLink`, `task.take` and `task.handBack`,
    /// which the other built-ins call; refused while off.
    pub async fn api(&self, method: &str, args: Value) -> Result<Value, ExtError> {
        if !self.is_on() {
            return Err(ExtError::new("off", OFF));
        }
        self.call(method, args, "builtin").await
    }

    /// The tools it adds, while it is on.
    pub fn mcp_tools(&self) -> Vec<Value> {
        if self.is_on() {
            serde_json::from_str(TOOLS).unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    /// An agent's call to one of its tools, while it is on.
    pub async fn mcp_call(&self, name: &str, arguments: Value) -> Option<Value> {
        if !self.is_on() || !self.manifest.mcp.iter().any(|t| t.name == name) {
            return None;
        }
        Some(
            match self.call(&format!("mcp.{name}"), arguments, "mcp").await {
                Ok(result) => result,
                Err(e) => {
                    json!({ "content": [{ "type": "text", "text": e.message }], "isError": true })
                }
            },
        )
    }

    /// Publishes the card badges again, as the board is now.
    pub async fn badges(&self) {
        if !self.is_on() {
            return;
        }
        if let Err(e) = self.call("badges", Value::Null, "client").await {
            warn!(error = %e.message, "[tasks] could not publish the card badges");
        }
    }
}

/// Tasks as an entry of the built-ins' registry.
pub fn entry(tasks: &Arc<TasksBuiltin>) -> Arc<dyn Builtin> {
    Arc::new(Entry(Arc::clone(tasks)))
}

struct Entry(Arc<TasksBuiltin>);

impl Builtin for Entry {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn events(&self) -> &'static [&'static str] {
        &[AGENT_CONVERSATION]
    }

    fn on_by_default(&self) -> bool {
        true
    }

    /// Publishes the badges, so a card shows its task from vornd's start.
    fn start<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        async move {
            self.0.set_on(true);
            self.0.badges().await;
        }
        .boxed()
    }

    fn handle<'a>(
        &'a self,
        _api: &'a Api,
        action: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        async move { self.0.call(action, args, "client").await }.boxed()
    }

    fn on_events<'a>(&'a self, api: &'a Api, batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async move { api.router.hand_bundled(&self.0.bundled(), &batch).await }.boxed()
    }

    fn switched(&self, on: bool) {
        self.0.set_on(on);
    }

    fn stop<'a>(&'a self, api: &'a Api) -> BoxFuture<'a, ()> {
        async move { api.router.stop(ID).await }.boxed()
    }

    fn mcp_tools(&self) -> Vec<Value> {
        self.0.mcp_tools()
    }

    fn mcp_call<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Option<Value>> {
        self.0.mcp_call(name, arguments).boxed()
    }
}

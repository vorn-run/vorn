//! Tasks' routes in vornd. The installed Tasks extension answers the
//! clients' `task:*` writes, the `task.*` calls Connectors and the engine
//! make, and its MCP task tools, through the router under the same gate and
//! lifecycle as any other. Uninstalled, each is refused, and the tasks stay
//! where they are, still listed by the core.
//!
//! What a component cannot know, vornd answers: where a task's image is on
//! this machine (`task:imageGetPath` answers the file's name, which becomes
//! its path here).

use serde_json::{json, Value};
use tracing::warn;
use vorn_extensions::api::types::CallCtx;
use vorn_extensions::api::ExtError;

use super::super::Extensions;
use crate::native::{Answer, Native};

/// The id Tasks goes by.
pub const ID: &str = "vorn.tasks";

/// Its name, as a refusal words it.
const NAME: &str = "Tasks";

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

/// Calls Tasks' `action` as `caller`, refused while it is not installed.
async fn call(native: &Native, action: &str, args: Value, caller: &str) -> Result<Value, ExtError> {
    let ext = native
        .extensions()
        .ok_or_else(|| ExtError::new("unsupported", "vornd is not hosting extensions"))?;
    if ext.router().program(ID).is_none() {
        return Err(ExtError::new("not-installed", super::not_installed(NAME)));
    }
    let ctx = CallCtx {
        caller: caller.to_owned(),
        project: None,
    };
    ext.call(ID, action, args, ctx).await
}

/// Answers one of its `task:*` calls.
pub async fn answer(native: &Native, method: &str, params: Value) -> Answer {
    let answered = match call(native, method, params, "client").await {
        Ok(v) => v,
        Err(e) => return Answer::Error(e.message),
    };
    match method {
        "task:imageGetPath" => {
            let name = answered.get("file").and_then(Value::as_str).unwrap_or("");
            match path_of(native, name) {
                Ok(path) => Answer::Result(json!(path)),
                Err(e) => Answer::Error(e),
            }
        }
        "task:imageDelete" | "task:imageCleanup" if answered.is_null() => Answer::Void,
        _ => Answer::Result(answered),
    }
}

/// Where Tasks' file `name` is on this machine.
fn path_of(native: &Native, name: &str) -> Result<String, String> {
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

/// `task.importItems`, `task.sourceLink`, `task.take` and `task.handBack`,
/// which Connectors and the engine call.
pub async fn api(native: &Native, method: &str, args: Value) -> Result<Value, ExtError> {
    call(native, method, args, "builtin").await
}

/// Publishes the card badges again, as the board is now; nothing while it is not installed.
pub async fn badges(native: &Native) {
    let installed = native
        .extensions()
        .is_some_and(|e| e.router().program(ID).is_some());
    if !installed {
        return;
    }
    if let Err(e) = call(native, "badges", Value::Null, "client").await {
        warn!(error = %e.message, "[tasks] could not publish the card badges");
    }
}

/// Whether `name` is one of the task tools Vorn lists while Tasks is installed.
pub fn offers(name: &str) -> bool {
    vorn_mcp::hosted_tools()
        .filter(|(id, _)| *id == ID)
        .flat_map(|(_, tools)| tools)
        .any(|t| t["name"] == name)
}

/// An agent's call to task tool `name`, answered by Tasks' `mcp.<name>`.
pub async fn mcp_call(ext: &Extensions, name: &str, arguments: Value) -> Value {
    let ctx = CallCtx {
        caller: "mcp".to_owned(),
        project: None,
    };
    match ext.call(ID, &format!("mcp.{name}"), arguments, ctx).await {
        Ok(result) => result,
        Err(e) => json!({ "content": [{ "type": "text", "text": e.message }], "isError": true }),
    }
}

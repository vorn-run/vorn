//! The core's routes to extensions Vorn ships as ordinary installs: an
//! agent's `browser:*` and `device:*` calls, its MCP tools and a client's
//! `ssh:*` reach the installed component through the router, under the same
//! gate and lifecycle as any other. Uninstalled, each call says so.

pub mod browser;
pub mod device;
pub mod ssh;

use serde_json::Value;
use vorn_extensions::api::types::CallCtx;

use crate::native::{Answer, Native};

/// A core route's `method` (as `browser:openPane`), answered by installed
/// extension `id`'s action of the same name; uninstalled, it says how to get `name`.
pub async fn answer(native: &Native, id: &str, name: &str, method: &str, params: Value) -> Answer {
    let Some(ext) = native.extensions() else {
        return Answer::Error("vornd is not hosting extensions".to_owned());
    };
    if ext.router().program(id).is_none() {
        return Answer::Error(not_installed(name));
    }
    match ext.call(id, method, params, client()).await {
        Ok(v) => Answer::Result(v),
        Err(e) => Answer::Error(e.message),
    }
}

/// What a call is told while extension `name` is not installed.
pub fn not_installed(name: &str) -> String {
    format!("{name} is not installed. Install it in Settings → Extensions.")
}

/// Vorn's own MCP tools that call the extensions `installed` says are, so
/// an agent is offered none whose extension is gone.
pub fn mcp_tools(installed: impl Fn(&str) -> bool) -> Vec<Value> {
    vorn_mcp::hosted_tools()
        .filter(|(id, _)| installed(id))
        .flat_map(|(_, tools)| tools.iter().cloned())
        .collect()
}

fn client() -> CallCtx {
    CallCtx {
        caller: "client".to_owned(),
        project: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(tools: &[Value]) -> Vec<&str> {
        tools.iter().filter_map(|t| t["name"].as_str()).collect()
    }

    #[test]
    fn an_extensions_tools_are_offered_only_while_it_is_installed() {
        assert!(mcp_tools(|_| false).is_empty());
        let device = mcp_tools(|id| id == device::ID);
        let device = names(&device);
        assert!(device.contains(&"device_list") && device.contains(&"open_device_pane"));
        assert!(!device.contains(&"read_page"));
        let both = mcp_tools(|id| id == device::ID || id == browser::ID);
        assert!(names(&both).contains(&"read_page"));
    }
}

//! MCP connections: one child per connection, spoken to through the Rust MCP
//! SDK, as the server's `connectors/mcp.ts` and `mcp-clients.ts` do.
//!
//! A connection's child starts on its first call and is kept until it exits.
//! The server stops its child when the connection is edited; here each call
//! compares what the child would start with now (command, arguments and the
//! connection's environment, secrets included) with what it started with,
//! and starts it again when they differ, so an edit made on any path takes
//! effect at the next call.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, JsonObject,
};
use rmcp::service::{Peer, RoleClient, RunningService, ServiceError};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};
use tracing::info;

use super::env::Env;

/// The MCP SDK's default request timeout, which the server's calls use.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// How a connection's child starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
    /// The connection's own variables, `env` then `secretEnv`, which win
    /// over the safe environment.
    pub env: Vec<(String, String)>,
}

impl Launch {
    /// `command`, `args` and `env` from a connection's filters and its
    /// secret variables, as `buildSpawnConfig` builds them for an MCP
    /// connection that names no package.
    pub fn of(
        filters: &Map<String, Value>,
        secret_env: &Map<String, Value>,
    ) -> Result<Launch, String> {
        let command = match filters.get("command") {
            None | Some(Value::Null) => String::new(),
            Some(v) => js_string(v),
        };
        let command = js_trim(&command).to_owned();
        if command.is_empty() {
            return Err("MCP connection is missing a command".into());
        }
        let args = match parse_json(filters.get("args")) {
            Some(Value::Array(items)) => items.iter().map(js_string).collect(),
            _ => Vec::new(),
        };
        let mut env = parse_json_object(filters.get("env"));
        for (k, v) in secret_env {
            env.insert(k.clone(), v.clone());
        }
        let env = env
            .into_iter()
            .map(|(k, v)| (k, v.as_str().map(str::to_owned).unwrap_or_default()))
            .collect();
        Ok(Launch { command, args, env })
    }
}

/// A running child, and what it started with.
struct Live {
    launch: Launch,
    service: RunningService<RoleClient, ClientConfig>,
}

/// Every connection's child.
#[derive(Default)]
pub struct McpClients {
    slots: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Option<Live>>>>>,
}

impl std::fmt::Debug for McpClients {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.slots.lock().map_or(0, |s| s.len());
        f.debug_struct("McpClients")
            .field("connections", &n)
            .finish()
    }
}

impl McpClients {
    /// The client of `id`'s child, started with `launch` on `base` (the safe
    /// environment) unless one is running with the same launch. Two calls
    /// for one connection share a start.
    pub async fn client(
        &self,
        id: &str,
        launch: &Launch,
        base: Env,
    ) -> Result<Peer<RoleClient>, String> {
        let slot = Arc::clone(
            self.slots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(id.to_owned())
                .or_default(),
        );
        let mut live = slot.lock().await;
        if let Some(running) = live.as_ref() {
            if running.launch == *launch && !running.service.is_transport_closed() {
                return Ok(running.service.peer().clone());
            }
            if running.service.is_transport_closed() {
                info!(connection = id, "an MCP child exited; starting it again");
            }
        }
        // The one it replaces ends as it drops.
        *live = None;
        let service = start(launch, base).await?;
        let peer = service.peer().clone();
        *live = Some(Live {
            launch: launch.clone(),
            service,
        });
        Ok(peer)
    }

    /// Ends `id`'s child, if it has one.
    pub fn stop(&self, id: &str) {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        if let Some(slot) = slot {
            tokio::spawn(async move {
                if let Some(live) = slot.lock().await.take() {
                    let _ = live.service.cancel().await;
                }
            });
        }
    }
}

/// Starts a child and completes the MCP handshake with it.
async fn start(
    launch: &Launch,
    base: Env,
) -> Result<RunningService<RoleClient, ClientConfig>, String> {
    let env = spawn_env(base, &launch.env);
    let program = resolve(&launch.command, &env);
    let mut command = vorn_spawn::tokio_command(&program);
    command.args(&launch.args).env_clear().envs(env);
    let (transport, _) = TokioChildProcess::builder(command)
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| spawn_error(&launch.command, &err))?;
    let config = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("vorn", "0.1.0"),
    );
    match tokio::time::timeout(REQUEST_TIMEOUT, config.serve(transport)).await {
        Ok(Ok(service)) => Ok(service),
        Ok(Err(err)) => Err(init_error(&err)),
        Err(_) => Err(TIMED_OUT.into()),
    }
}

const TIMED_OUT: &str = "MCP error -32001: Request timed out";
const CLOSED: &str = "MCP error -32000: Connection closed";

/// What a child is started with: the variables the MCP SDK passes on by
/// default, then the safe environment, then the connection's own.
fn spawn_env(base: Env, own: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = default_inherited();
    let mut put = |k: String, v: String| match env.iter_mut().find(|(key, _)| *key == k) {
        Some(slot) => slot.1 = v,
        None => env.push((k, v)),
    };
    for (k, v) in base {
        put(k, v);
    }
    for (k, v) in own {
        put(k.clone(), v.clone());
    }
    env
}

/// The MCP SDK's `getDefaultEnvironment`: a few variables from this
/// process, skipping any that hold a shell function.
fn default_inherited() -> Vec<(String, String)> {
    let names: &[&str] = if cfg!(windows) {
        &[
            "APPDATA",
            "HOMEDRIVE",
            "HOMEPATH",
            "LOCALAPPDATA",
            "PATH",
            "PROCESSOR_ARCHITECTURE",
            "SYSTEMDRIVE",
            "SYSTEMROOT",
            "TEMP",
            "USERNAME",
            "USERPROFILE",
            "PROGRAMFILES",
        ]
    } else {
        &["HOME", "LOGNAME", "PATH", "SHELL", "TERM", "USER"]
    };
    names
        .iter()
        .filter_map(|name| {
            let value = std::env::var(name).ok()?;
            (!value.starts_with("()")).then(|| ((*name).to_owned(), value))
        })
        .collect()
}

/// The program to run. On Unix the child's PATH is searched as Node's spawn
/// searches it; on Windows a bare name is looked up on that PATH with the
/// extensions `PATHEXT` names, as cross-spawn does, so `npx` finds `npx.cmd`.
pub fn resolve(command: &str, env: &[(String, String)]) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let var = |name: &str| {
            env.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        let path = std::path::Path::new(command);
        if path.components().count() == 1 && path.extension().is_none() {
            let exts = var("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
            for dir in var("PATH")
                .unwrap_or_default()
                .split(';')
                .filter(|d| !d.is_empty())
            {
                for ext in exts.split(';').filter(|e| !e.is_empty()) {
                    let full = std::path::Path::new(dir).join(format!("{command}{ext}"));
                    if full.is_file() {
                        return full;
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = env;
    std::path::PathBuf::from(command)
}

/// A start that failed, worded as Node's spawn words it.
fn spawn_error(command: &str, err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::NotFound => format!("spawn {command} ENOENT"),
        std::io::ErrorKind::PermissionDenied => format!("spawn {command} EACCES"),
        _ => format!("spawn {command} {err}"),
    }
}

fn init_error(err: &rmcp::service::ClientInitializeError) -> String {
    use rmcp::service::ClientInitializeError as E;
    match err {
        E::ConnectionClosed(_) => CLOSED.into(),
        E::JsonRpcError(data) => format!("MCP error {}: {}", data.code.0, data.message),
        other => other.to_string(),
    }
}

/// A failed request, worded as the MCP SDK's `McpError` words it.
fn call_error(err: &ServiceError) -> String {
    match err {
        ServiceError::McpError(data) => format!("MCP error {}: {}", data.code.0, data.message),
        ServiceError::TransportClosed | ServiceError::TransportSend(_) => CLOSED.into(),
        ServiceError::Timeout { .. } => TIMED_OUT.into(),
        other => other.to_string(),
    }
}

/// `tools/list`, one page, each tool as the server stores it: `name`, and
/// `title`, `description`, `inputSchema` and `outputSchema` when present.
pub async fn discover(peer: &Peer<RoleClient>) -> Result<Vec<Value>, String> {
    let listed = match tokio::time::timeout(REQUEST_TIMEOUT, peer.list_tools(None)).await {
        Ok(Ok(listed)) => listed,
        Ok(Err(err)) => return Err(call_error(&err)),
        Err(_) => return Err(TIMED_OUT.into()),
    };
    let mut tools = Vec::with_capacity(listed.tools.len());
    for tool in listed.tools {
        let raw = serde_json::to_value(&tool).map_err(|e| e.to_string())?;
        let mut out = Map::new();
        out.insert(
            "name".into(),
            raw.get("name").cloned().unwrap_or(Value::Null),
        );
        if let Some(Value::String(title)) = raw.get("title") {
            if !title.is_empty() {
                out.insert("title".into(), json!(title));
            }
        }
        for key in ["description", "inputSchema", "outputSchema"] {
            if let Some(v) = raw.get(key).filter(|v| js_truthy(v)) {
                out.insert(key.into(), v.clone());
            }
        }
        tools.push(Value::Object(out));
    }
    Ok(tools)
}

/// `invokeMcpTool`: calls `name` with `args` coerced by the tool's stored
/// input schema, and answers in the shape every action answers in.
pub async fn invoke(
    peer: &Peer<RoleClient>,
    name: &str,
    args: &Map<String, Value>,
    tools: &[Value],
) -> Value {
    let schema = tools
        .iter()
        .find(|t| t.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|t| t.get("inputSchema"));
    let arguments = coerce_args(schema, args);
    let mut params = CallToolRequestParams::new(name.to_owned());
    params.arguments = Some(arguments);
    let result = match tokio::time::timeout(REQUEST_TIMEOUT, peer.call_tool(params)).await {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => return json!({ "success": false, "error": call_error(&err) }),
        Err(_) => return json!({ "success": false, "error": TIMED_OUT }),
    };
    let result = match serde_json::to_value(&result) {
        Ok(v) => v,
        Err(err) => return json!({ "success": false, "error": err.to_string() }),
    };
    let output = match result.get("structuredContent") {
        Some(structured) if !structured.is_null() => structured.clone(),
        _ => result.clone(),
    };
    if result.get("isError").is_some_and(js_truthy) {
        let error = extract_text_error(result.get("content"))
            .unwrap_or_else(|| format!("MCP tool {name} reported an error"));
        json!({ "success": false, "error": error, "output": output })
    } else {
        json!({ "success": true, "output": output })
    }
}

/// The first text block's text, which is what a failed tool says went wrong.
fn extract_text_error(content: Option<&Value>) -> Option<String> {
    content?.as_array()?.iter().find_map(|block| {
        (block.get("type").and_then(Value::as_str) == Some("text"))
            .then(|| block.get("text").and_then(Value::as_str))
            .flatten()
            .map(str::to_owned)
    })
}

/// `coerceMcpArgs`: string form values turned back into the types the tool's
/// input schema declares. Values that do not convert pass through, so the
/// MCP server's own validation reports them.
pub fn coerce_args(schema: Option<&Value>, args: &Map<String, Value>) -> JsonObject {
    let Some(schema) = schema.filter(|s| !s.is_null()) else {
        return args.clone();
    };
    let properties = schema_properties(schema);
    if properties.is_empty() {
        return args.clone();
    }
    let mut out = JsonObject::new();
    for (key, value) in args {
        let t = schema_type_hint(properties.iter().find(|(k, _)| k == key).map(|(_, v)| v));
        let Value::String(text) = value else {
            out.insert(key.clone(), value.clone());
            continue;
        };
        let is = |name: &str| matches!(&t, Some(Value::String(s)) if s == name);
        if text.is_empty() && t.is_some() && !is("string") && !schema_required(schema, key) {
            continue;
        }
        let coerced = if is("number") || is("integer") {
            if text.is_empty() {
                value.clone()
            } else {
                js_number(text).unwrap_or_else(|| value.clone())
            }
        } else if is("boolean") {
            match text.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => value.clone(),
            }
        } else if is("object") || is("array") {
            if text.is_empty() {
                value.clone()
            } else {
                serde_json::from_str::<Value>(text).unwrap_or_else(|_| value.clone())
            }
        } else {
            value.clone()
        };
        out.insert(key.clone(), coerced);
    }
    out
}

/// `mcpToolToConnectorAction`: a stored tool as an action the workflow
/// editor draws a form for. `None` for a tool whose fields the server would
/// fail on.
pub fn tool_action(tool: &Value) -> Option<Value> {
    let input = tool.get("inputSchema").filter(|s| !s.is_null());
    let mut fields = Vec::new();
    for (key, raw) in input.map(schema_properties).unwrap_or_default() {
        let prop = raw.as_object();
        let get = |k: &str| prop.and_then(|p| p.get(k));
        let declared = schema_type_hint(Some(&raw));
        let mut field = Map::new();
        field.insert("key".into(), json!(key));
        field.insert("label".into(), json!(key));
        field.insert(
            "required".into(),
            json!(input.is_some_and(|s| schema_required(s, &key))),
        );
        if let Some(d) = get("description").filter(|d| js_truthy(d)) {
            field.insert("description".into(), d.clone());
        }
        if let Some(default) = get("default") {
            field.insert("placeholder".into(), json!(default.to_string()));
        }
        field.insert("supportsTemplates".into(), json!(true));
        match get("enum") {
            Some(Value::Array(options)) if !options.is_empty() => {
                field.insert("type".into(), json!("select"));
                let options: Vec<Value> = options
                    .iter()
                    .map(|v| json!({ "value": js_string(v), "label": js_string(v) }))
                    .collect();
                field.insert("options".into(), Value::Array(options));
            }
            _ => match declared.as_ref().and_then(Value::as_str) {
                Some("object" | "array") => {
                    field.insert("type".into(), json!("textarea"));
                    field.insert("placeholder".into(), json!("{} or []"));
                }
                _ => {
                    field.insert("type".into(), json!("text"));
                }
            },
        }
        fields.push(Value::Object(field));
    }
    let name = tool.get("name").cloned().unwrap_or(Value::Null);
    let label = match tool.get("title") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) => Some(js_trim(t).to_owned()).filter(|t| !t.is_empty()),
        // `title?.trim()` on anything else throws in the server.
        Some(_) => return None,
    };
    let mut action = Map::new();
    action.insert("type".into(), name.clone());
    action.insert("label".into(), label.map_or(name, Value::String));
    if let Some(d) = tool.get("description").filter(|d| js_truthy(d)) {
        action.insert("description".into(), d.clone());
    }
    action.insert("configFields".into(), Value::Array(fields));
    if let Some(o) = tool.get("outputSchema").filter(|o| js_truthy(o)) {
        action.insert("outputSchema".into(), o.clone());
    }
    Some(Value::Object(action))
}

/// `schemaProperties`: the schema's `properties` as key and value pairs,
/// `Object.entries` of whatever object it is (an array's are its indexes).
fn schema_properties(schema: &Value) -> Vec<(String, Value)> {
    match schema.get("properties") {
        Some(Value::Object(props)) => props.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

/// `schemaTypeHint`: a property's `type`, the first that is not `"null"`
/// when it is a list. `None` is `undefined`.
fn schema_type_hint(prop: Option<&Value>) -> Option<Value> {
    let prop = prop?;
    if !prop.is_object() {
        return None;
    }
    match prop.get("type")? {
        // `t.find(x => x !== 'null') ?? t[0]`: a JSON null found is nullish
        // too, so `t[0]` is the answer then.
        Value::Array(types) => match types.iter().find(|t| t.as_str() != Some("null")) {
            Some(Value::Null) | None => types.first().cloned(),
            found => found.cloned(),
        },
        t => Some(t.clone()),
    }
}

/// `schemaRequired`: whether `required` lists `key`.
fn schema_required(schema: &Value, key: &str) -> bool {
    schema
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(|req| req.iter().any(|r| r.as_str() == Some(key)))
}

/// `JSON.parse` of a string param, `None` for anything else or what does not
/// parse.
fn parse_json(raw: Option<&Value>) -> Option<Value> {
    match raw {
        Some(Value::String(text)) if !text.is_empty() => serde_json::from_str(text).ok(),
        _ => None,
    }
}

/// `parseJsonObject`: a JSON object in a string, each value made a string.
pub fn parse_json_object(raw: Option<&Value>) -> Map<String, Value> {
    match parse_json(raw) {
        Some(Value::Object(obj)) => obj
            .into_iter()
            .map(|(k, v)| (k, Value::String(js_string(&v))))
            .collect(),
        _ => Map::new(),
    }
}

/// JavaScript truthiness.
pub fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// JavaScript's `String(value)` for a JSON value.
pub fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => i.to_string(),
            (_, Some(u), _) => u.to_string(),
            (_, _, Some(f)) => js_number_string(f),
            _ => n.to_string(),
        },
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// How JavaScript prints a number.
fn js_number_string(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        return "0".into();
    }
    let abs = f.abs();
    if (1e-6..1e21).contains(&abs) {
        return format!("{f}");
    }
    // Rust prints `1e21` and `1e-7`; JavaScript writes the sign: `1e+21`.
    let s = format!("{f:e}");
    match s.split_once('e') {
        Some((mantissa, exp)) if !exp.starts_with('-') => format!("{mantissa}e+{exp}"),
        _ => s,
    }
}

/// `Number(text)` when it is finite, as a JSON number.
fn js_number(text: &str) -> Option<Value> {
    let t = js_trim(text);
    let f = if t.is_empty() {
        0.0
    } else if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()? as f64
    } else if let Some(oct) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        u64::from_str_radix(oct, 8).ok()? as f64
    } else if let Some(bin) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        u64::from_str_radix(bin, 2).ok()? as f64
    } else {
        // Rust also reads `inf`, `nan` and `infinity`, which `Number` does not.
        if t.bytes()
            .any(|b| b.is_ascii_alphabetic() && b != b'e' && b != b'E')
        {
            return None;
        }
        t.parse::<f64>().ok()?
    };
    if !f.is_finite() {
        return None;
    }
    // An integer stays an integer, as JSON.stringify writes it.
    if f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
        return Some(json!(f as i64));
    }
    serde_json::Number::from_f64(f).map(Value::Number)
}

/// JavaScript's `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn builds_the_launch_as_the_server_does() {
        let launch = Launch::of(
            &obj(json!({
                "command": "  npx ",
                "args": "[\"-y\", 1, true]",
                "env": "{\"A\":\"1\",\"B\":2}"
            })),
            &obj(json!({ "B": "secret", "C": "c" })),
        )
        .unwrap();
        assert_eq!(launch.command, "npx");
        assert_eq!(launch.args, ["-y", "1", "true"]);
        assert_eq!(
            launch.env,
            [
                ("A".to_owned(), "1".to_owned()),
                ("B".to_owned(), "secret".to_owned()),
                ("C".to_owned(), "c".to_owned())
            ]
        );
        assert_eq!(
            Launch::of(&obj(json!({ "command": " " })), &Map::new()).unwrap_err(),
            "MCP connection is missing a command"
        );
        let bare = Launch::of(
            &obj(json!({ "command": "x", "args": "{}", "env": "nope" })),
            &Map::new(),
        )
        .unwrap();
        assert!(bare.args.is_empty() && bare.env.is_empty());
    }

    #[test]
    fn the_connection_wins_over_the_safe_environment() {
        let env = spawn_env(
            vec![("PATH".into(), "/safe".into()), ("X".into(), "base".into())],
            &[("X".into(), "own".into())],
        );
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("PATH"), Some("/safe"));
        assert_eq!(get("X"), Some("own"));
    }

    #[test]
    fn coerces_form_values_by_the_input_schema() {
        let schema = json!({
            "properties": {
                "n": { "type": "integer" },
                "f": { "type": ["null", "number"] },
                "b": { "type": "boolean" },
                "o": { "type": "object" },
                "s": { "type": "string" },
                "req": { "type": "number" }
            },
            "required": ["req"]
        });
        let args = obj(json!({
            "n": "42", "f": "1.5", "b": "true", "o": "{\"a\":1}", "s": "", "req": "",
            "bad": "x", "num": 3, "notbool": "yes", "badjson": "{"
        }));
        let out = Value::Object(coerce_args(Some(&schema), &args));
        assert_eq!(
            out,
            json!({
                "n": 42, "f": 1.5, "b": true, "o": { "a": 1 }, "s": "", "req": "",
                "bad": "x", "num": 3, "notbool": "yes", "badjson": "{"
            })
        );
        // An optional non-string field left blank is dropped.
        let dropped = coerce_args(Some(&schema), &obj(json!({ "n": "", "b": "" })));
        assert!(dropped.is_empty());
        assert_eq!(
            coerce_args(None, &obj(json!({ "n": "1" }))),
            obj(json!({ "n": "1" }))
        );
        assert_eq!(js_number("0x1f"), Some(json!(31)));
        assert_eq!(js_number(" 2e3 "), Some(json!(2000)));
        assert_eq!(js_number("Infinity"), None);
        assert_eq!(js_number("inf"), None);
        assert_eq!(js_number("1.5.2"), None);
    }

    #[test]
    fn draws_a_tool_as_an_action() {
        let tool = json!({
            "name": "echo",
            "title": "  Echo ",
            "description": "Says it back",
            "inputSchema": {
                "properties": {
                    "text": { "type": "string", "description": "Anything" },
                    "count": { "type": "integer", "default": 1 },
                    "tags": { "type": "array", "default": ["a"] },
                    "mode": { "type": "string", "enum": ["a", 3, null] }
                },
                "required": ["text"]
            },
            "outputSchema": { "type": "object" }
        });
        assert_eq!(
            tool_action(&tool).unwrap(),
            json!({
                "type": "echo",
                "label": "Echo",
                "description": "Says it back",
                "configFields": [
                    { "key": "text", "label": "text", "required": true, "description": "Anything", "supportsTemplates": true, "type": "text" },
                    { "key": "count", "label": "count", "required": false, "placeholder": "1", "supportsTemplates": true, "type": "text" },
                    { "key": "tags", "label": "tags", "required": false, "placeholder": "{} or []", "supportsTemplates": true, "type": "textarea" },
                    { "key": "mode", "label": "mode", "required": false, "supportsTemplates": true, "type": "select",
                      "options": [{ "value": "a", "label": "a" }, { "value": "3", "label": "3" }, { "value": "null", "label": "null" }] }
                ],
                "outputSchema": { "type": "object" }
            })
        );
        let bare = tool_action(&json!({ "name": "x", "title": "" })).unwrap();
        assert_eq!(
            bare,
            json!({ "type": "x", "label": "x", "configFields": [] })
        );
        assert_eq!(tool_action(&json!({ "name": "x", "title": 3 })), None);
    }

    #[test]
    fn prints_values_as_javascript_does() {
        assert_eq!(js_string(&json!(1.5)), "1.5");
        assert_eq!(js_string(&json!(1e21)), "1e+21");
        assert_eq!(js_string(&json!(1e-7)), "1e-7");
        assert_eq!(js_string(&json!([1, null, "a"])), "1,,a");
        assert_eq!(js_string(&json!({})), "[object Object]");
        assert_eq!(js_string(&json!(null)), "null");
        assert!(!js_truthy(&json!("")) && js_truthy(&json!({})) && !js_truthy(&json!(0)));
    }

    #[test]
    fn a_failed_tool_says_what_its_first_text_block_says() {
        assert_eq!(
            extract_text_error(Some(
                &json!([{ "type": "image" }, { "type": "text", "text": "no" }])
            )),
            Some("no".into())
        );
        assert_eq!(extract_text_error(Some(&json!("x"))), None);
    }

    #[test]
    fn words_a_failed_start_as_node_does() {
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(spawn_error("nope", &err), "spawn nope ENOENT");
    }
}

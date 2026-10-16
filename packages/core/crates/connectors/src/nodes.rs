//! The workflow step Connectors declares (`[[contributes.nodes]]`):
//! `callConnectorAction` runs one action of a connection with the step's
//! arguments, already resolved against the run, and parks the step when the
//! connection must sign in again.

use std::future::Future;

use serde_json::{json, Map, Value};
use vorn_work::js;

fn error_of(result: &Value) -> Option<String> {
    result
        .get("error")
        .filter(|e| vorn_work::is_truthy(Some(e)))
        .map(js::to_string)
}

/// The step a node type answers, by its id; `None` for a type Connectors does not declare.
pub async fn run<F, Fut>(node_type: &str, request: &Value, call: F) -> Option<Value>
where
    F: Fn(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    match node_type {
        "callConnectorAction" => Some(call_action(request, call).await),
        _ => None,
    }
}

/// `callConnectorAction`: the action's answer as the step's outputs.
pub async fn call_action<F, Fut>(request: &Value, call: F) -> Value
where
    F: Fn(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let raw = request.get("raw").unwrap_or(&Value::Null);
    let resolved = request.get("config").unwrap_or(&Value::Null);
    let action = raw.get("action").map(js::to_string).unwrap_or_default();
    let mut args = Map::new();
    for (k, v) in resolved
        .get("args")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        args.insert(
            k.clone(),
            Value::String(v.as_str().map_or_else(|| js::to_string(v), str::to_owned)),
        );
    }
    let mut params = Map::new();
    if let Some(c) = raw.get("connectionId") {
        params.insert("connectionId".into(), c.clone());
    }
    if let Some(a) = raw.get("action") {
        params.insert("action".into(), a.clone());
    }
    params.insert("args".into(), Value::Object(args));
    let result = match call("connection:executeAction", Value::Object(params)).await {
        Ok(result) => result,
        Err(message) => return json!({ "status": "error", "error": message }),
    };
    let success = result
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let logs = js::stringify_pretty(&result);
    let error = error_of(&result);
    let mut out = Map::new();
    if !success && result.get("errorKind").and_then(Value::as_str) == Some("needs-sign-in") {
        out.insert("status".into(), json!("waiting"));
        out.insert("waitingFor".into(), json!("signIn"));
        out.insert("logs".into(), json!(logs));
        if let Some(e) = error {
            out.insert("error".into(), json!(e));
        }
        return Value::Object(out);
    }
    out.insert(
        "status".into(),
        json!(if success { "success" } else { "error" }),
    );
    out.insert(
        "output".into(),
        json!(format!(
            "{action} {}",
            if success { "succeeded" } else { "failed" }
        )),
    );
    out.insert("logs".into(), json!(logs));
    if let Some(o) = result.get("output").filter(|o| o.is_object()) {
        out.insert("structuredOutput".into(), o.clone());
    }
    if let Some(e) = error {
        out.insert("error".into(), json!(e));
    }
    Value::Object(out)
}

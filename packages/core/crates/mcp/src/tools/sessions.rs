//! `tools/sessions.ts`: terminal and headless sessions, over the server's RPCs.

use serde_json::{json, Value};

use super::{error_of, failed, items, object, pretty, text, Args, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;

/// The fields of a session an agent is shown.
fn pick(session: &Value, keys: &[&'static str]) -> Result<Value, String> {
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        out.push((*key, json::prop(Some(session), key)?.cloned()));
    }
    Ok(object(out))
}

pub async fn list_sessions<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let run = async {
        if args.str("filter").unwrap_or("active") == "active" {
            let mut sessions = items(Some(&cx.call("terminal:listActive", None).await?)).to_vec();
            if let Some(name) = args.nonempty("project_name") {
                sessions.retain(|s| s.get("projectName").and_then(Value::as_str) == Some(name));
            }
            let summary = sessions
                .iter()
                .map(|s| {
                    pick(
                        s,
                        &[
                            "id",
                            "agentType",
                            "projectName",
                            "status",
                            "displayName",
                            "branch",
                            "pid",
                        ],
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<Value, String>(pretty(&Value::Array(summary)))
        } else {
            let sessions = cx
                .call("sessions:getRecent", args.get("project_path").cloned())
                .await?;
            Ok(pretty(&sessions))
        }
    };
    Ok(run.await.unwrap_or_else(error_of))
}

pub async fn launch_session<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let payload = object([
        ("agentType", args.get("agent_type").cloned()),
        ("projectName", args.get("project_name").cloned()),
        ("projectPath", args.get("project_path").cloned()),
        ("initialPrompt", args.nonempty("prompt").map(Value::from)),
        ("branch", args.nonempty("branch").map(Value::from)),
        (
            "useWorktree",
            args.truthy("use_worktree").then_some(Value::Bool(true)),
        ),
        (
            "displayName",
            args.nonempty("display_name").map(Value::from),
        ),
    ]);
    let headless = args.truthy("headless");
    let (method, label) = if headless {
        ("headless:create", "headless")
    } else {
        ("terminal:create", "terminal")
    };
    let run = async {
        let session = cx.call(method, Some(payload)).await?;
        pick(
            &session,
            &["id", "agentType", "projectName", "pid", "status"],
        )
    };
    Ok(match run.await {
        Ok(summary) => pretty(&summary),
        Err(err) => failed(format!("Error launching {label} agent: {err}")),
    })
}

pub async fn kill_session<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let (method, label) = if args.truthy("headless") {
        ("headless:kill", "headless session")
    } else {
        ("terminal:kill", "session")
    };
    let id = json::display(args.get("id"));
    Ok(match cx.call(method, args.get("id").cloned()).await {
        Ok(_) => text(format!("Killed {label}: {id}")),
        Err(err) => failed(format!("Error killing {label}: {err}")),
    })
}

pub async fn rename_session<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let params = object([
        ("id", args.get("id").cloned()),
        ("displayName", args.get("display_name").cloned()),
    ]);
    Ok(match cx.call("terminal:rename", Some(params)).await {
        Ok(_) => text(format!(
            "Renamed session {} to \"{}\"",
            json::display(args.get("id")),
            json::display(args.get("display_name"))
        )),
        Err(err) => failed(format!("Error renaming session: {err}")),
    })
}

pub async fn reorder_sessions<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let ids = args.get("session_ids").cloned();
    let count = items(ids.as_ref()).len();
    Ok(match cx.call("terminal:reorder", ids).await {
        Ok(_) => text(format!("Reordered {count} sessions")),
        Err(err) => failed(format!("Error reordering sessions: {err}")),
    })
}

/// `output.join('\n')`, and what it throws when the answer is not a list.
fn join_lines(output: &Value) -> Result<String, String> {
    match output {
        Value::Array(lines) => Ok(json::join(lines, "\n")),
        Value::Null => Err("Cannot read properties of null (reading 'join')".to_owned()),
        _ => Err("output.join is not a function".to_owned()),
    }
}

pub async fn read_session_output<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let params = object([
        ("id", args.get("id").cloned()),
        ("lines", args.get("lines").cloned()),
    ]);
    let run = async {
        let output = cx.call("terminal:readOutput", Some(params)).await?;
        join_lines(&output)
    };
    Ok(match run.await {
        Ok(lines) => text(lines),
        Err(err) => failed(format!("Error reading session output: {err}")),
    })
}

/// `data.replace(/[\r\n]+$/, '')`.
fn without_trailing_newlines(data: &str) -> &str {
    data.trim_end_matches(['\r', '\n'])
}

pub async fn write_to_terminal<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let given = args.str("data").unwrap_or_default();
    let data = if args.truthy("raw") {
        given.to_owned()
    } else {
        format!("{}\r", without_trailing_newlines(given))
    };
    let params = object([
        ("id", args.get("id").cloned()),
        ("data", Some(Value::from(data))),
    ]);
    Ok(match cx.notify("terminal:write", Some(params)).await {
        Ok(()) => text(format!(
            "Wrote to session: {}",
            json::display(args.get("id"))
        )),
        Err(err) => failed(format!("Error writing to terminal: {err}")),
    })
}

/// Named keys and the bytes they send, in the TypeScript's order.
const KEY_MAP: [(&str, &str); 17] = [
    ("enter", "\r"),
    ("escape", "\x1b"),
    ("esc", "\x1b"),
    ("tab", "\x09"),
    ("shift+tab", "\x1b[Z"),
    ("up", "\x1b[A"),
    ("down", "\x1b[B"),
    ("left", "\x1b[D"),
    ("right", "\x1b[C"),
    ("backspace", "\x7f"),
    ("delete", "\x1b[3~"),
    ("home", "\x1b[H"),
    ("end", "\x1b[F"),
    ("ctrl+c", "\x03"),
    ("ctrl+d", "\x04"),
    ("ctrl+x", "\x18"),
    ("ctrl+z", "\x1a"),
];

/// What `KEY_MAP[key]` finds, `None` when it finds nothing true.
///
/// `KEY_MAP` is a plain object, so two lower-case names reach its prototype:
/// `constructor` (a function, which JSON leaves out) and `__proto__` (an empty
/// object). The port sends what the TypeScript sends for them.
fn mapped(key: &str) -> Option<Option<Value>> {
    match key {
        "constructor" => Some(None),
        "__proto__" => Some(Some(json!({}))),
        _ => KEY_MAP
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, bytes)| Some(Value::from(*bytes))),
    }
}

pub async fn send_key<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let given = args.str("key").unwrap_or_default();
    let lowered = given.to_lowercase();
    let key = json::trim(&lowered);
    let data = match mapped(key) {
        Some(data) => data,
        None => {
            let ctrl = key
                .strip_prefix("ctrl+")
                .filter(|rest| rest.len() == 1 && rest.as_bytes()[0].is_ascii_lowercase());
            if let Some(letter) = ctrl {
                let code = letter.as_bytes()[0].to_ascii_uppercase() - 64;
                Some(Value::from(char::from(code).to_string()))
            } else if json::utf16_len(given) == 1 {
                Some(Value::from(given))
            } else {
                let names: Vec<&str> = KEY_MAP.iter().map(|(name, _)| *name).collect();
                return Ok(failed(format!(
                    "Unknown key: \"{given}\". Supported: single chars (1, y, n), named keys ({}), or ctrl+<letter>.",
                    names.join(", ")
                )));
            }
        }
    };
    let params = object([("id", args.get("id").cloned()), ("data", data)]);
    Ok(match cx.notify("terminal:write", Some(params)).await {
        Ok(()) => text(format!(
            "Sent key \"{given}\" to session: {}",
            json::display(args.get("id"))
        )),
        Err(err) => failed(format!("Error sending key to terminal: {err}")),
    })
}

pub async fn list_session_events<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let limit = args.get("limit").cloned().unwrap_or_else(|| json!(50));
    let call = if let Some(session) = args.nonempty("session_id") {
        cx.call(
            "sessionEvent:listBySession",
            Some(json!({ "sessionId": session, "limit": limit })),
        )
        .await
    } else {
        cx.call(
            "sessionEvent:list",
            Some(object([
                ("eventType", args.get("event_type").cloned()),
                ("limit", Some(limit)),
            ])),
        )
        .await
    };
    Ok(match call {
        Ok(events) => pretty(&events),
        Err(err) => failed(format!("Error listing session events: {err}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_a_trailing_run_of_newlines() {
        assert_eq!(without_trailing_newlines("a\r\n\n"), "a");
        assert_eq!(without_trailing_newlines("a\nb"), "a\nb");
    }

    #[test]
    fn prototype_names_send_what_javascript_sends() {
        assert_eq!(mapped("constructor"), Some(None));
        assert_eq!(mapped("__proto__"), Some(Some(json!({}))));
        assert_eq!(mapped("tab"), Some(Some(json!("\t"))));
        assert_eq!(mapped("nope"), None);
    }
}

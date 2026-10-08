//! A permission request: what clients are shown (`PermissionRequestInfo`)
//! and the answer the agent's hook is given.

use serde_json::{json, Map, Value};

use crate::event::Event;

/// The terminal a request is about, as clients are shown it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct About {
    pub terminal: String,
    pub agent_type: Option<String>,
    pub project_name: Option<String>,
}

/// What clients are shown of request `request_id`.
pub fn info(request_id: &str, event: &Event, about: &About) -> Value {
    let input = event
        .get("tool_input")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let tool = event.tool_name().unwrap_or("unknown");
    let mut out = Map::new();
    out.insert("requestId".into(), json!(request_id));
    out.insert("sessionId".into(), json!(event.session()));
    out.insert("terminalId".into(), json!(about.terminal));
    out.insert("toolName".into(), json!(tool));
    out.insert("toolInput".into(), input.clone());
    let description = ["file_path", "command", "description"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str));
    if let Some(d) = description {
        out.insert("description".into(), json!(d));
    }
    if let Some(a) = &about.agent_type {
        out.insert("agentType".into(), json!(a));
    }
    if let Some(p) = &about.project_name {
        out.insert("projectName".into(), json!(p));
    }
    if let Some(s) = event.get("permission_suggestions") {
        out.insert("permissionSuggestions".into(), s.clone());
    }
    if tool == "AskUserQuestion" {
        if let Some(q) = input.get("questions") {
            out.insert("questions".into(), q.clone());
        }
    }
    Value::Object(out)
}

/// The hook's answer: allow or deny, with the permissions and input a
/// person changed when they allowed it.
pub fn decision(
    allow: bool,
    updated_permissions: Option<&Value>,
    updated_input: Option<&Value>,
) -> String {
    let mut decision = Map::new();
    decision.insert(
        "behavior".into(),
        json!(if allow { "allow" } else { "deny" }),
    );
    if allow {
        if let Some(p) = updated_permissions.filter(|p| p.as_array().is_some_and(|a| !a.is_empty()))
        {
            decision.insert("updatedPermissions".into(), p.clone());
        }
        if let Some(i) = updated_input.filter(|i| !i.is_null()) {
            decision.insert("updatedInput".into(), i.clone());
        }
    }
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_a_request_and_words_its_answer() {
        let body = json!({
            "hook_event_name": "PermissionRequest", "session_id": "s", "cwd": "/p",
            "tool_name": "AskUserQuestion",
            "tool_input": { "command": "ls", "questions": [{ "q": 1 }] },
            "permission_suggestions": [{ "type": "x" }],
        });
        let event = Event::parse(body.to_string().as_bytes(), None).unwrap();
        let about = About {
            terminal: "t".into(),
            agent_type: Some("claude".into()),
            project_name: None,
        };
        let shown = info("r", &event, &about);
        assert_eq!(shown["description"], "ls");
        assert_eq!(shown["questions"], json!([{ "q": 1 }]));
        assert_eq!(shown["terminalId"], "t");
        assert!(shown.get("projectName").is_none());

        let bare = Event::parse(
            br#"{"hook_event_name":"PermissionRequest","session_id":"s","cwd":""}"#,
            None,
        )
        .unwrap();
        let shown = info("r", &bare, &about);
        assert_eq!(
            (shown["toolName"].as_str(), &shown["toolInput"]),
            (Some("unknown"), &json!({}))
        );

        assert_eq!(
            decision(false, Some(&json!([1])), None),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny"}}}"#
        );
        let allowed: Value =
            serde_json::from_str(&decision(true, Some(&json!([1])), Some(&json!({ "a": 1 }))))
                .unwrap();
        assert_eq!(
            allowed["hookSpecificOutput"]["decision"],
            json!({ "behavior": "allow", "updatedPermissions": [1], "updatedInput": { "a": 1 } })
        );
        let empty: Value = serde_json::from_str(&decision(true, Some(&json!([])), None)).unwrap();
        assert_eq!(
            empty["hookSpecificOutput"]["decision"],
            json!({ "behavior": "allow" })
        );
    }
}

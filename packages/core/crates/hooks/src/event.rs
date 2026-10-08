//! A hook event as an agent posts it (`HookEvent`): its name, the
//! conversation it belongs to and the directory it ran in, and the terminal
//! it comes from when Vorn started the agent. Every other field is kept as
//! the agent sent it.

use serde_json::{Map, Value};

/// The header Claude's hook fills with the terminal's id (`VORN_SESSION_ID`).
pub const TERMINAL_HEADER: &str = "x-vorn-terminal";

/// A posted event whose name, conversation and directory are there.
#[derive(Clone, Debug, PartialEq)]
pub struct Event(Map<String, Value>);

/// Why a posted body is not an event, in a line short enough to log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Malformed(pub String);

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn nonempty(map: &Map<String, Value>, key: &str) -> bool {
    map.get(key)
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
}

impl Event {
    /// Reads a posted body. The terminal is taken from `header` when it names
    /// one, else from the body's `vorn_terminal_id`; an empty one is none, as
    /// an agent outside Vorn interpolates.
    pub fn parse(body: &[u8], header: Option<&str>) -> Result<Event, Malformed> {
        let parsed: Value =
            serde_json::from_slice(body).map_err(|e| Malformed(format!("not JSON: {e}")))?;
        let Value::Object(mut map) = parsed else {
            let kind = match parsed {
                Value::Array(_) => "array",
                Value::String(_) => "string",
                Value::Number(_) => "number",
                Value::Bool(_) => "boolean",
                _ => "null",
            };
            return Err(Malformed(format!("not an object ({kind})")));
        };
        // An empty cwd is a Copilot event's, which is linked without one.
        let has_cwd = map.get("cwd").is_some_and(Value::is_string);
        if !nonempty(&map, "hook_event_name") || !nonempty(&map, "session_id") || !has_cwd {
            let mut missing: Vec<&str> = ["hook_event_name", "session_id"]
                .into_iter()
                .filter(|k| !nonempty(&map, k))
                .collect();
            if !has_cwd {
                missing.push("cwd");
            }
            let name = map
                .get("hook_event_name")
                .and_then(Value::as_str)
                .filter(|n| !n.is_empty())
                .unwrap_or("no name");
            return Err(Malformed(format!("{name} missing {}", missing.join(", "))));
        }
        let from_body = map
            .remove("vorn_terminal_id")
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|t| !t.is_empty());
        let terminal = header
            .filter(|h| !h.is_empty())
            .map(str::to_owned)
            .or(from_body);
        if let Some(terminal) = terminal {
            map.insert("vorn_terminal_id".into(), Value::String(terminal));
        }
        Ok(Event(map))
    }

    fn text(&self, key: &str) -> &str {
        self.0.get(key).and_then(Value::as_str).unwrap_or("")
    }

    /// `hook_event_name`: `SessionStart`, `PreToolUse`, `PermissionRequest`, ...
    pub fn name(&self) -> &str {
        self.text("hook_event_name")
    }

    /// The agent's conversation (`session_id`).
    pub fn session(&self) -> &str {
        self.text("session_id")
    }

    pub fn cwd(&self) -> &str {
        self.text("cwd")
    }

    /// The Vorn terminal the agent runs in, when it says.
    pub fn terminal(&self) -> Option<&str> {
        self.0.get("vorn_terminal_id").and_then(Value::as_str)
    }

    pub fn tool_name(&self) -> Option<&str> {
        self.0
            .get("tool_name")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Ends a pending permission request of the conversation: the agent went on.
    pub fn dismisses_permissions(&self) -> bool {
        matches!(
            self.name(),
            "PostToolUse" | "PostToolUseFailure" | "Stop" | "UserPromptSubmit"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_event_and_the_terminal_it_names() {
        let body =
            br#"{"hook_event_name":"Stop","session_id":"s","cwd":"/p","vorn_terminal_id":"body"}"#;
        let e = Event::parse(body, Some("head")).unwrap();
        assert_eq!(
            (e.name(), e.session(), e.cwd(), e.terminal()),
            ("Stop", "s", "/p", Some("head"))
        );
        assert_eq!(
            Event::parse(body, Some("")).unwrap().terminal(),
            Some("body")
        );
        let outside =
            br#"{"hook_event_name":"Stop","session_id":"s","cwd":"","vorn_terminal_id":""}"#;
        let e = Event::parse(outside, None).unwrap();
        assert_eq!((e.terminal(), e.get("vorn_terminal_id")), (None, None));
    }

    #[test]
    fn says_what_a_malformed_body_lacks() {
        let missing = |b: &[u8]| Event::parse(b, None).unwrap_err().0;
        assert_eq!(
            missing(br#"{"hook_event_name":"Notification"}"#),
            "Notification missing session_id, cwd"
        );
        assert_eq!(
            missing(br#"{"hook_event_name":"","session_id":"s","cwd":"/"}"#),
            "no name missing hook_event_name"
        );
        assert_eq!(missing(b"[1]"), "not an object (array)");
        assert!(missing(b"{").starts_with("not JSON"));
    }
}

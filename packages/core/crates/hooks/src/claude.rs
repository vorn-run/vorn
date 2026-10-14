//! Vorn's hook entries in Claude Code's settings (`~/.claude/settings.json`).
//!
//! One HTTP hook per event, marked by its `X-Vorn` header so it can be told
//! from the person's own hooks, which are never touched. Each carries the
//! endpoint's token and names the terminal from `VORN_SESSION_ID`, which
//! Claude fills only because `allowedEnvVars` lists it.

use std::io;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::event::TERMINAL_HEADER;

const VORN_HEADER: &str = "X-Vorn";

/// The events Vorn listens to.
pub const EVENTS: [&str; 8] = [
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "Stop",
    "Notification",
    "PermissionRequest",
    "SessionStart",
    "SessionEnd",
];

/// The events Vorn still needs from a Claude Code that reports its own
/// status (OSC 7501): the conversation it starts, to resume it, and its
/// permission requests, to answer them from Vorn.
pub const ANSWER_EVENTS: [&str; 2] = ["SessionStart", "PermissionRequest"];

/// The first Claude Code that reports its own status.
const REPORTS_SINCE: [u32; 3] = [2, 1, 295];

/// Whether the Claude Code that printed `version` reports its own status.
pub fn reports_status(version: &str) -> bool {
    let mut parts = version
        .trim()
        .split(|c: char| !c.is_ascii_digit())
        .skip_while(|p| p.is_empty())
        .take(3)
        .map(|p| p.parse::<u32>().unwrap_or(0));
    let v = [0; 3].map(|_| parts.next().unwrap_or(0));
    v >= REPORTS_SINCE
}

/// The events to install hooks for.
pub fn events(reports_status: bool) -> &'static [&'static str] {
    if reports_status {
        &ANSWER_EVENTS
    } else {
        &EVENTS
    }
}

fn entry(port: u16, token: &str) -> Value {
    json!({
        "type": "http",
        "url": format!("http://localhost:{port}/hooks"),
        "headers": {
            VORN_HEADER: "true",
            "Authorization": format!("Bearer {token}"),
            TERMINAL_HEADER: "$VORN_SESSION_ID",
        },
        "allowedEnvVars": ["VORN_SESSION_ID"],
        "timeout": 30,
    })
}

fn is_vorn(hook: &Value) -> bool {
    hook.get("headers")
        .and_then(|h| h.get(VORN_HEADER))
        .and_then(Value::as_str)
        == Some("true")
}

/// An event's entries without Vorn's hooks; an entry left with none goes,
/// and with `drop_empty` so does one that never had any.
fn without_vorn(entries: Vec<Value>, drop_empty: bool) -> Vec<Value> {
    entries
        .into_iter()
        .filter_map(|mut entry| {
            let hooks: Vec<Value> = entry
                .get("hooks")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let had_vorn = hooks.iter().any(is_vorn);
            let kept: Vec<Value> = hooks.into_iter().filter(|h| !is_vorn(h)).collect();
            if kept.is_empty() && (had_vorn || drop_empty) {
                return None;
            }
            if let Some(map) = entry.as_object_mut() {
                map.insert("hooks".into(), Value::Array(kept));
            }
            Some(entry)
        })
        .collect()
}

fn hooks_of(settings: &mut Value) -> &mut Map<String, Value> {
    if !settings.is_object() {
        *settings = json!({});
    }
    let map = settings.as_object_mut().expect("made an object above");
    if !map.get("hooks").is_some_and(Value::is_object) {
        map.insert("hooks".into(), json!({}));
    }
    map.get_mut("hooks")
        .and_then(Value::as_object_mut)
        .expect("made an object above")
}

/// `settings` with Vorn's hooks for `events` pointing at `port`, any
/// earlier ones of Vorn's removed.
pub fn install(mut settings: Value, port: u16, token: &str, events: &[&str]) -> Value {
    let hooks = hooks_of(&mut settings);
    for event in EVENTS {
        let existing = hooks.get(event).and_then(Value::as_array).cloned();
        let had = existing.is_some();
        let mut kept = without_vorn(existing.unwrap_or_default(), false);
        if events.contains(&event) {
            kept.push(json!({ "hooks": [entry(port, token)] }));
        }
        if kept.is_empty() && !had {
            continue;
        }
        if kept.is_empty() {
            hooks.remove(event);
        } else {
            hooks.insert(event.into(), Value::Array(kept));
        }
    }
    settings
}

/// `settings` without Vorn's hooks; an event left with no entries goes.
pub fn uninstall(mut settings: Value) -> Value {
    let hooks = hooks_of(&mut settings);
    for event in EVENTS {
        let existing = hooks
            .get(event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let kept = without_vorn(existing, true);
        if kept.is_empty() {
            hooks.remove(event);
        } else {
            hooks.insert(event.into(), Value::Array(kept));
        }
    }
    settings
}

/// Installs into the settings file, read as `{}` when absent or unreadable.
pub fn install_file(file: &Path, port: u16, token: &str, events: &[&str]) -> io::Result<()> {
    let settings = std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(&install(settings, port, token, events))
        .map_err(io::Error::other)?;
    std::fs::write(file, text)
}

/// Uninstalls from the settings file; a file that is absent or not JSON is left alone.
pub fn uninstall_file(file: &Path) -> io::Result<()> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Ok(());
    };
    let Ok(settings) = serde_json::from_str::<Value>(&text) else {
        return Ok(());
    };
    let text = serde_json::to_string_pretty(&uninstall(settings)).map_err(io::Error::other)?;
    std::fs::write(file, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mine() -> Value {
        json!({ "type": "command", "command": "echo mine" })
    }

    #[test]
    fn installs_beside_the_person_s_hooks_and_replaces_its_own() {
        let settings = json!({
            "model": "x",
            "hooks": { "Stop": [
                { "hooks": [mine()] },
                { "hooks": [entry(1, "old")] },
                { "matcher": "*", "hooks": [mine(), entry(1, "old")] },
            ]}
        });
        let installed = install(settings, 56432, "tok", &EVENTS);
        let stop = installed["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 3);
        assert_eq!(stop[1], json!({ "matcher": "*", "hooks": [mine()] }));
        assert_eq!(stop[2]["hooks"][0]["url"], "http://localhost:56432/hooks");
        assert_eq!(
            stop[2]["hooks"][0]["headers"]["Authorization"],
            "Bearer tok"
        );
        assert_eq!(
            stop[2]["hooks"][0]["headers"][TERMINAL_HEADER],
            "$VORN_SESSION_ID"
        );
        assert_eq!(installed["model"], "x");
        assert_eq!(installed["hooks"].as_object().unwrap().len(), EVENTS.len());

        let removed = uninstall(installed);
        assert_eq!(
            removed["hooks"],
            json!({ "Stop": [{ "hooks": [mine()] }, { "matcher": "*", "hooks": [mine()] }] })
        );
    }

    #[test]
    fn a_claude_reporting_its_status_keeps_only_the_hooks_it_needs_answered() {
        assert!(!reports_status("2.1.294 (Claude Code)"));
        assert!(reports_status("2.1.295 (Claude Code)"));
        assert!(reports_status("v2.2.0"));
        assert!(!reports_status("1.9.999"));
        assert!(!reports_status(""));

        let full = install(
            json!({ "hooks": { "Stop": [{ "hooks": [mine()] }] } }),
            1,
            "t",
            &EVENTS,
        );
        let trimmed = install(full, 1, "t", events(true));
        let hooks = trimmed["hooks"].as_object().unwrap();
        let mut names: Vec<&str> = hooks.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["PermissionRequest", "SessionStart", "Stop"]);
        assert_eq!(hooks["Stop"], json!([{ "hooks": [mine()] }]));
        assert!(is_vorn(&hooks["PermissionRequest"][0]["hooks"][0]));
    }

    #[test]
    fn keeps_the_settings_file_whole_through_install_and_uninstall() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude/settings.json");
        install_file(&file, 9, "t", &EVENTS).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\n  \"hooks\": {"), "{text}");
        uninstall_file(&file).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "{\n  \"hooks\": {}\n}"
        );
        std::fs::write(&file, "not json").unwrap();
        uninstall_file(&file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json");
    }
}

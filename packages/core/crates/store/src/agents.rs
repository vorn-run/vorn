//! How the agents are configured, read by a second process.
//!
//! vornd answers the calls that look the agents up (which are installed,
//! which models one offers), and both depend on the configured commands and
//! on the variables passed through to what an agent runs. It reads them from
//! the server's file as [`crate::ProjectHosts`] reads the projects: read-only,
//! fresh on every call, creating and migrating nothing, so a save the server
//! has just made is what the next call sees. A session vornd starts reads the
//! shell it runs in from here too.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Map, Value};

use crate::config::load_agent_commands;
use crate::Result;

/// The agents' configuration as the server's `loadConfig` reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentSettings {
    /// `agentCommands`' stored rows, by agent type, each an
    /// `AgentCommandConfig`. An agent with no row uses its default.
    pub commands: Map<String, Value>,
    /// `defaults.envPassthrough`, the names kept: strings only, as stored.
    pub env_passthrough: Vec<String>,
    /// `defaults.shell`, the shell sessions run in; `None` for the default.
    pub shell: Option<String>,
    /// `defaults.minimalShellPrompt`: a shell's own prompt is left out.
    pub minimal_shell_prompt: bool,
}

impl AgentSettings {
    /// Reads them from the database at `path`. `None` when there is no such
    /// file or the server has not created its tables yet; an error when a
    /// row does not parse, so the caller cannot mistake it for no settings.
    pub fn read(path: &Path) -> Result<Option<AgentSettings>> {
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let tables: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('agent_commands', 'defaults')",
            [],
            |row| row.get(0),
        )?;
        if tables < 2 {
            return Ok(None);
        }
        let commands = load_agent_commands(&conn)?;
        let env_passthrough = match default_value(&conn, "envPassthrough")? {
            Some(Value::Array(items)) => items
                .into_iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let shell = match default_value(&conn, "shell")? {
            Some(Value::String(s)) => Some(s),
            _ => None,
        };
        let minimal_shell_prompt =
            default_value(&conn, "minimalShellPrompt")? == Some(Value::Bool(true));
        Ok(Some(AgentSettings {
            commands,
            env_passthrough,
            shell,
            minimal_shell_prompt,
        }))
    }
}

/// One stored default, as the JSON it is kept as.
fn default_value(conn: &Connection, key: &str) -> Result<Option<Value>> {
    let stored: Option<String> = conn
        .query_row("SELECT value FROM defaults WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(match stored {
        Some(text) => Some(serde_json::from_str::<Value>(&text)?),
        None => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_commands_and_passthrough_and_nothing_from_an_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("vorn.db");
        assert_eq!(AgentSettings::read(&db).unwrap(), None);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE defaults (key TEXT PRIMARY KEY, value TEXT)")
            .unwrap();
        assert_eq!(AgentSettings::read(&db).unwrap(), None);
        conn.execute_batch(
            r#"CREATE TABLE agent_commands (agent_type TEXT PRIMARY KEY, command TEXT NOT NULL,
                 args TEXT NOT NULL, headless_args TEXT, fallback_command TEXT, fallback_args TEXT,
                 row_revision INTEGER);
               INSERT INTO agent_commands VALUES ('gemini', 'gem', '["--x"]', NULL, 'gem2', NULL, 1);
               INSERT INTO defaults VALUES ('envPassthrough', '["ANTHROPIC_API_KEY", 3]');"#,
        )
        .unwrap();
        let read = AgentSettings::read(&db).unwrap().unwrap();
        assert_eq!(read.env_passthrough, ["ANTHROPIC_API_KEY"]);
        assert_eq!((read.shell, read.minimal_shell_prompt), (None, false));
        conn.execute_batch(
            r#"INSERT INTO defaults VALUES ('shell', '"/bin/sh"');
               INSERT INTO defaults VALUES ('minimalShellPrompt', 'true');"#,
        )
        .unwrap();
        let read = AgentSettings::read(&db).unwrap().unwrap();
        assert_eq!(
            (read.shell.as_deref(), read.minimal_shell_prompt),
            (Some("/bin/sh"), true)
        );
        assert_eq!(
            read.commands["gemini"],
            serde_json::json!({ "command": "gem", "args": ["--x"], "fallbackCommand": "gem2" })
        );
        conn.execute_batch("UPDATE agent_commands SET args = 'not json'")
            .unwrap();
        assert!(AgentSettings::read(&db).is_err());
    }
}

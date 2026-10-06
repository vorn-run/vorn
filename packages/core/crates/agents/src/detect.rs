//! Which agents are installed (`agent-detector`): an agent is when its
//! configured command, or its fallback command, is a program on PATH, as
//! `which` (`where` on Windows) finds it.
//!
//! The server asks `which` once per command and keeps the answer until the
//! configuration is saved. This looks on PATH itself, which costs a few
//! `stat`s, and keeps nothing: an agent installed meanwhile is found.

use std::path::Path;

use crate::{Agent, AgentCommand};

/// Whether each agent is installed, in [`Agent::ALL`]'s order. `command_of`
/// gives the agent's configured command; `path_env` is the PATH to search.
pub fn installed(
    mut command_of: impl FnMut(Agent) -> AgentCommand,
    path_env: Option<&str>,
) -> Vec<(Agent, bool)> {
    Agent::ALL
        .into_iter()
        .map(|agent| {
            let config = command_of(agent);
            let found = command_exists(&config.command, path_env)
                || config
                    .fallback_command
                    .as_deref()
                    .is_some_and(|fallback| command_exists(fallback, path_env));
            (agent, found)
        })
        .collect()
}

/// Whether `which name` succeeds: a name with a separator is a path to a
/// program, any other is looked for in each directory of `path_env`.
pub fn command_exists(name: &str, path_env: Option<&str>) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.contains('/') || (cfg!(windows) && name.contains('\\')) {
        return program(Path::new(name));
    }
    let Some(path_env) = path_env else {
        return false;
    };
    let sep = if cfg!(windows) { ';' } else { ':' };
    let suffixes: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ".com", ""]
    } else {
        &[""]
    };
    path_env
        .split(sep)
        .filter(|dir| !dir.is_empty())
        .any(|dir| {
            suffixes
                .iter()
                .any(|s| program(&Path::new(dir).join(format!("{name}{s}"))))
        })
}

/// A regular file that can be run.
#[cfg(unix)]
fn program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn program(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn tool(dir: &Path, name: &str, mode: u32) {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn finds_a_command_or_its_fallback_on_path() {
        let dir = tempfile::tempdir().unwrap();
        tool(dir.path(), "claude", 0o755);
        tool(dir.path(), "codex", 0o644);
        tool(dir.path(), "gem-fallback", 0o755);
        std::fs::create_dir(dir.path().join("copilot")).unwrap();
        let path = format!("/nonexistent::{}", dir.path().display());
        let found = installed(
            |agent| {
                let mut config = agent.default_command();
                if agent == Agent::Gemini {
                    config.command = "gemini-missing".into();
                    config.fallback_command = Some("gem-fallback".into());
                }
                config
            },
            Some(&path),
        );
        assert_eq!(
            found,
            [
                (Agent::Claude, true),
                (Agent::Copilot, false),
                (Agent::Codex, false),
                (Agent::OpenCode, false),
                (Agent::Gemini, true),
            ]
        );
    }

    #[test]
    fn reads_a_command_with_a_separator_as_its_path() {
        let dir = tempfile::tempdir().unwrap();
        tool(dir.path(), "agent", 0o755);
        let full = dir.path().join("agent");
        assert!(command_exists(full.to_str().unwrap(), None));
        assert!(!command_exists("npx claude", Some("/usr/bin")));
        assert!(!command_exists("", Some("/usr/bin")));
    }
}

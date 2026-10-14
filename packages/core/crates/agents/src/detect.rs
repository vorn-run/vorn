//! Which agents are installed (`agent-detector`): an agent is when its
//! configured command, or its fallback command, is a program on PATH, as
//! `which` (`where` on Windows) finds it.
//!
//! The server asks `which` once per command and keeps the answer until the
//! configuration is saved. This looks on PATH itself, which costs a few
//! `stat`s, and keeps nothing: an agent installed meanwhile is found.

use std::path::{Path, PathBuf};

use crate::{Agent, AgentCommand};

/// Whether each agent is installed, in [`Agent::ALL`]'s order. `command_of`
/// gives the agent's configured command; `path_env` is the PATH to search.
pub fn installed(
    command_of: impl FnMut(Agent) -> AgentCommand,
    path_env: Option<&str>,
) -> Vec<(Agent, bool)> {
    locate(command_of, path_env)
        .into_iter()
        .map(|(agent, at)| (agent, at.is_some()))
        .collect()
}

/// Where each agent's command, or else its fallback, is, in [`Agent::ALL`]'s order.
pub fn locate(
    mut command_of: impl FnMut(Agent) -> AgentCommand,
    path_env: Option<&str>,
) -> Vec<(Agent, Option<PathBuf>)> {
    Agent::ALL
        .into_iter()
        .map(|agent| {
            let config = command_of(agent);
            let at = command_path(&config.command, path_env).or_else(|| {
                config
                    .fallback_command
                    .as_deref()
                    .and_then(|fallback| command_path(fallback, path_env))
            });
            (agent, at)
        })
        .collect()
}

/// Whether `which name` succeeds.
pub fn command_exists(name: &str, path_env: Option<&str>) -> bool {
    command_path(name, path_env).is_some()
}

/// What `which name` prints: a name with a separator is a path to a
/// program, any other is looked for in each directory of `path_env`.
pub fn command_path(name: &str, path_env: Option<&str>) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.contains('/') || (cfg!(windows) && name.contains('\\')) {
        return program(Path::new(name)).then(|| PathBuf::from(name));
    }
    let sep = if cfg!(windows) { ';' } else { ':' };
    let suffixes: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ".com", ""]
    } else {
        &[""]
    };
    path_env?
        .split(sep)
        .filter(|dir| !dir.is_empty())
        .flat_map(|dir| {
            suffixes
                .iter()
                .map(move |s| Path::new(dir).join(format!("{name}{s}")))
        })
        .find(|p| program(p))
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

    #[test]
    fn locates_the_first_directory_holding_the_command() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        tool(a.path(), "claude", 0o644);
        tool(b.path(), "claude", 0o755);
        let path = format!("{}:{}", a.path().display(), b.path().display());
        let found = locate(|agent| agent.default_command(), Some(&path));
        assert_eq!(found[0], (Agent::Claude, Some(b.path().join("claude"))));
        assert!(found[1..].iter().all(|(_, at)| at.is_none()));
    }
}

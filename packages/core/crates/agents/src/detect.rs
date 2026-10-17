//! Which agents are installed: an agent is when its configured command is a
//! program on PATH, as `which` (`where` on Windows) finds it.
//!
//! Looks on PATH itself on every call, which costs a few `stat`s and keeps
//! nothing: an agent installed meanwhile is found.

use std::path::{Path, PathBuf};

use crate::{Agent, AgentCommand, Agents};

/// Whether each agent is installed, in `agents`' order. `command_of` gives
/// the agent's configured command; `path_env` is the PATH to search.
pub fn installed(
    agents: &Agents,
    command_of: impl FnMut(&Agent) -> AgentCommand,
    path_env: Option<&str>,
) -> Vec<(Agent, bool)> {
    locate(agents, command_of, path_env)
        .into_iter()
        .map(|(agent, at)| (agent, at.is_some()))
        .collect()
}

/// Where each agent's command is, in `agents`' order.
pub fn locate(
    agents: &Agents,
    mut command_of: impl FnMut(&Agent) -> AgentCommand,
    path_env: Option<&str>,
) -> Vec<(Agent, Option<PathBuf>)> {
    agents
        .iter()
        .map(|agent| {
            let config = command_of(agent);
            (agent.clone(), command_path(&config.command, path_env))
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
    use crate::testing::bare;

    fn agents() -> Agents {
        Agents::new(["one", "two", "three", "four", "five"].map(bare))
    }

    fn tool(dir: &Path, name: &str, mode: u32) {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn finds_a_command_on_path() {
        let dir = tempfile::tempdir().unwrap();
        tool(dir.path(), "one", 0o755);
        tool(dir.path(), "three", 0o644);
        tool(dir.path(), "five-renamed", 0o755);
        std::fs::create_dir(dir.path().join("two")).unwrap();
        let path = format!("/nonexistent::{}", dir.path().display());
        let agents = agents();
        let found: Vec<(String, bool)> = installed(
            &agents,
            |agent| {
                let mut config = agent.default_command();
                if agent.id() == "five" {
                    config.command = "five-renamed".into();
                }
                config
            },
            Some(&path),
        )
        .into_iter()
        .map(|(a, at)| (a.id().to_owned(), at))
        .collect();
        let want = [
            ("one", true),
            ("two", false),
            ("three", false),
            ("four", false),
            ("five", true),
        ]
        .map(|(id, at)| (id.to_owned(), at));
        assert_eq!(found, want);
    }

    #[test]
    fn reads_a_command_with_a_separator_as_its_path() {
        let dir = tempfile::tempdir().unwrap();
        tool(dir.path(), "agent", 0o755);
        let full = dir.path().join("agent");
        assert!(command_exists(full.to_str().unwrap(), None));
        assert!(!command_exists("npx agent", Some("/usr/bin")));
        assert!(!command_exists("", Some("/usr/bin")));
    }

    #[test]
    fn locates_the_first_directory_holding_the_command() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        tool(a.path(), "one", 0o644);
        tool(b.path(), "one", 0o755);
        let path = format!("{}:{}", a.path().display(), b.path().display());
        let found = locate(&agents(), Agent::default_command, Some(&path));
        assert_eq!(found[0], (bare("one"), Some(b.path().join("one"))));
        assert!(found[1..].iter().all(|(_, at)| at.is_none()));
    }
}

//! The `agent:*` calls and `sessions:getRecent`: what the server reads about
//! the coding agents without starting one, answered by `vorn_agents`.
//!
//! The agents' configured commands and the names passed through to what they
//! run are the server's settings, read fresh from its database on each call
//! that needs them; a call vornd cannot read them for goes to the server.
//! Past sessions need no settings, only the agents' own files and the
//! project's worktrees.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use tracing::debug;
use vorn_agents::history::{self, Homes, ProjectScope};
use vorn_agents::models::{check_model_command, DiscoveryError, ModelRequest};
use vorn_agents::{detect, Agent, AgentCommand, ProbeContext};
use vorn_git::repo::Git;
use vorn_store::AgentSettings;

use super::{bad_params, env, Answer, Native};

/// Answers `method` with `params`.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    match method {
        "agent:detectInstalled" => detect_installed(native),
        "agent:listModels" => list_models(native, params),
        "sessions:getRecent" => recent_sessions(native, params),
        _ => Answer::Forward,
    }
}

/// The error for settings vornd could not read.
pub(super) fn no_settings() -> Answer {
    Answer::Error("vornd could not read the settings".to_owned())
}

/// The error for a stored agent command of a shape the server's code would not read.
pub(super) fn unreadable_command(agent: Agent) -> Answer {
    Answer::Error(format!(
        "the {} command in the settings cannot be read",
        agent.id()
    ))
}

/// The settings, or why vornd cannot answer without them.
pub(super) fn settings(native: &Native) -> Option<AgentSettings> {
    let db = native.db.get()?;
    match AgentSettings::read(db) {
        Ok(settings) => Some(settings.unwrap_or_default()),
        Err(err) => {
            debug!(%err, "could not read the agent settings; the server answers");
            None
        }
    }
}

/// The configured command for `agent`, or its default when none is stored.
/// `None` for a stored one of a shape the server's code would not expect.
pub(super) fn command_of(settings: &AgentSettings, agent: Agent) -> Option<AgentCommand> {
    let Some(stored) = settings.commands.get(agent.id()) else {
        return Some(agent.default_command());
    };
    let strings = |v: Option<&Value>| -> Option<Option<Vec<String>>> {
        match v {
            None | Some(Value::Null) => Some(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|i| i.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .map(Some),
            Some(_) => None,
        }
    };
    let fallback_command = match stored.get("fallbackCommand") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return None,
    };
    Some(AgentCommand {
        command: stored.get("command")?.as_str()?.to_owned(),
        args: strings(stored.get("args"))??,
        headless_args: strings(stored.get("headlessArgs"))?,
        fallback_command,
        fallback_args: strings(stored.get("fallbackArgs"))?,
    })
}

fn detect_installed(native: &Native) -> Answer {
    let Some(settings) = settings(native) else {
        return no_settings();
    };
    let mut commands = HashMap::with_capacity(Agent::ALL.len());
    for agent in Agent::ALL {
        match command_of(&settings, agent) {
            Some(command) => commands.insert(agent, command),
            None => return unreadable_command(agent),
        };
    }
    let env = native.env.get();
    let found = detect::installed(
        |agent| {
            commands
                .remove(&agent)
                .unwrap_or_else(|| agent.default_command())
        },
        path_of(&env),
    );
    let mut out = Map::new();
    for (agent, installed) in found {
        out.insert(agent.id().to_owned(), Value::Bool(installed));
    }
    Answer::Result(Value::Object(out))
}

/// `env.PATH ?? env.Path`.
fn path_of(env: &env::Env) -> Option<&str> {
    let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    get("PATH").or_else(|| get("Path"))
}

/// JavaScript truthiness, for the flags a request carries.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn list_models(native: &Native, params: &Value) -> Answer {
    let Value::Object(request) = params else {
        return bad_params("agent:listModels");
    };
    let project_path = match request.get("projectPath") {
        Some(Value::String(s)) => s.clone(),
        v if !truthy(v) => String::new(),
        // Not a string: the server's handler throws on it.
        _ => return bad_params("agent:listModels"),
    };
    let request = ModelRequest {
        agent: request
            .get("agentType")
            .and_then(Value::as_str)
            .and_then(Agent::from_id),
        project_path,
        remote: truthy(request.get("remoteHostId")),
        refresh: truthy(request.get("refresh")),
    };
    // Answered without settings when there is nothing to ask; a relative
    // project would be resolved against the server's directory, not vornd's.
    let askable = request.agent.is_some_and(Agent::selects_models)
        && !request.remote
        && !request.project_path.is_empty()
        && !request.project_path.contains("{{");
    let (agent, command, env) = match request.agent {
        Some(agent) if askable => {
            if !Path::new(&request.project_path).is_absolute() {
                return bad_params("agent:listModels");
            }
            let Some(settings) = settings(native) else {
                return no_settings();
            };
            let Some(command) = command_of(&settings, agent) else {
                return unreadable_command(agent);
            };
            let data_dir = native.db.get().and_then(|db| db.parent());
            let env = native.env.launch(&settings.env_passthrough, data_dir);
            (agent, command, env)
        }
        // Never asked: the catalog answers these unavailable before it looks
        // at the command.
        _ => (Agent::Claude, Agent::Claude.default_command(), Vec::new()),
    };
    let discover = {
        let (command, cwd) = (command.clone(), PathBuf::from(&request.project_path));
        move || {
            check_model_command(&command.command)?;
            let program = if Path::new(&command.command).is_absolute() {
                PathBuf::from(&command.command)
            } else {
                path_of(&env)
                    .and_then(|path| env::find_on_path(&command.command, path))
                    .ok_or_else(|| DiscoveryError::NotInstalled(command.command.clone()))?
            };
            vorn_agents::probe(
                &ProbeContext {
                    command: program,
                    args: command.args.clone(),
                    cwd,
                    env,
                },
                agent,
            )
        }
    };
    let answer = native.catalog.list(&request, &command, now_ms, discover);
    Answer::Result(answer.to_json())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn recent_sessions(native: &Native, params: &Value) -> Answer {
    let project = match params {
        Value::String(s) if !s.is_empty() => Some(s.as_str()),
        v if !truthy(Some(v)) => None,
        // Not a path: the server's handler throws on it.
        _ => return bad_params("sessions:getRecent"),
    };
    let Some(homes) = Homes::from_env() else {
        return Answer::Result(Value::Array(Vec::new()));
    };
    let scope = match project {
        Some(project) if !Path::new(project).is_absolute() => {
            return bad_params("sessions:getRecent")
        }
        Some(project) => {
            let git = Git {
                bin: native.env.git_bin(),
                env: native.env.get(),
                ssh: None,
            };
            let worktrees = git.list_worktrees(Path::new(project));
            Some(ProjectScope::new(
                project,
                worktrees.iter().map(|w| w.path.as_str()),
            ))
        }
        None => None,
    };
    let list = history::recent_sessions(&homes, scope.as_ref(), history::DEFAULT_LIMIT);
    Answer::Result(Value::Array(
        list.iter().map(history::RecentSession::to_json).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn settings_with(commands: Value) -> AgentSettings {
        AgentSettings {
            commands: commands.as_object().cloned().unwrap_or_default(),
            ..AgentSettings::default()
        }
    }

    #[test]
    fn reads_a_stored_command_and_defaults_the_rest() {
        let s = settings_with(json!({
            "gemini": { "command": "gem", "args": ["-x"], "fallbackCommand": "g2" },
            "codex": { "command": "codex", "args": "not a list" }
        }));
        let gemini = command_of(&s, Agent::Gemini).unwrap();
        assert_eq!(gemini.command, "gem");
        assert_eq!(gemini.args, ["-x"]);
        assert_eq!(gemini.fallback_command.as_deref(), Some("g2"));
        assert_eq!(
            command_of(&s, Agent::Claude),
            Some(Agent::Claude.default_command())
        );
        assert_eq!(command_of(&s, Agent::Codex), None);
    }

    #[test]
    fn answers_what_needs_no_settings_and_refuses_the_rest() {
        let native = Native::new();
        let root = if cfg!(windows) { "C:\\p" } else { "/p" };
        // Nothing to ask: answered whatever the database.
        let gemini = native.call(
            "agent:listModels",
            &json!({ "agentType": "gemini", "projectPath": root }),
        );
        assert!(
            matches!(gemini, Answer::Result(ref v) if v["status"] == "unavailable"),
            "{gemini:?}"
        );
        // To ask, the configured command is needed, and there is no database.
        assert_eq!(
            native.call(
                "agent:listModels",
                &json!({ "agentType": "claude", "projectPath": root })
            ),
            no_settings()
        );
        let bad = bad_params("agent:listModels");
        assert_eq!(
            native.call(
                "agent:listModels",
                &json!({ "agentType": "claude", "projectPath": "rel" })
            ),
            bad
        );
        assert_eq!(native.call("agent:listModels", &json!(null)), bad);
        assert_eq!(
            native.call("agent:detectInstalled", &json!(null)),
            no_settings()
        );
        let recent = bad_params("sessions:getRecent");
        assert_eq!(native.call("sessions:getRecent", &json!(5)), recent);
        assert_eq!(native.call("sessions:getRecent", &json!("rel")), recent);
    }
}

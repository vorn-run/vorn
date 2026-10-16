//! The `agent:*` calls and `sessions:getRecent`: what vornd reads about
//! the coding agents without starting one, answered by `vorn_agents` from
//! the agents' adapters (`agent:list`).
//!
//! The agents' configured commands and the names passed through to what they
//! run are settings, read fresh from the database on each call.
//! Past sessions need no settings, only the agents' own files and the
//! project's worktrees.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use tracing::debug;
use vorn_agents::history::{self, Homes, ProjectScope};
use vorn_agents::models::{check_model_command, CatalogAnswer, DiscoveryError, ModelRequest};
use vorn_agents::{detect, Agent, AgentCommand, ProbeContext};
use vorn_git::repo::Git;
use vorn_store::AgentSettings;

use super::{bad_params, env, Answer, Native};

/// Answers `method` with `params`.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    match method {
        "agent:list" => Answer::Result(native.adapters().describe()),
        "agent:detectInstalled" => detect_installed(native, params),
        "agent:locate" => locate(native, params),
        "agent:listModels" => list_models(native, params),
        "sessions:getRecent" => recent_sessions(native, params),
        _ => Answer::Unanswered,
    }
}

/// The error for settings vornd could not read.
pub(super) fn no_settings() -> Answer {
    Answer::Error("vornd could not read the settings".to_owned())
}

/// The error for a stored agent command of an unreadable shape.
pub(super) fn unreadable_command(agent: &Agent) -> Answer {
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
            debug!(%err, "could not read the agent settings");
            None
        }
    }
}

/// The configured command for `agent`, or its default when none is stored.
/// `None` for a stored one of an unexpected shape.
pub(super) fn command_of(settings: &AgentSettings, agent: &Agent) -> Option<AgentCommand> {
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
    Some(AgentCommand {
        command: stored.get("command")?.as_str()?.to_owned(),
        args: strings(stored.get("args"))??,
        headless_args: strings(stored.get("headlessArgs"))?,
    })
}

/// How long detection waits for the login shell: the first answer after vornd starts should already be right.
const DETECT_WAIT: Duration = Duration::from_secs(5);

/// How long a refresh waits for the shell asked again.
const REFRESH_WAIT: Duration = Duration::from_secs(15);

/// Whether `method` waits for the login shell first, which [`prepare`] does.
pub(super) fn waits_for_shell(method: &str) -> bool {
    matches!(method, "agent:locate" | "agent:detectInstalled")
}

/// Waits for the login shell to answer, or the wait to run out, before a
/// detection; `{ refresh: true }` asks the shell again first.
pub(super) fn prepare(native: &Native, params: &Value) {
    let wait = if truthy(params.get("refresh")) {
        native.env.refresh();
        REFRESH_WAIT
    } else {
        native.env.prime();
        DETECT_WAIT
    };
    native.env.settle(wait);
}

/// Where each agent's command is on the environment agents get.
fn found(native: &Native, _params: &Value) -> Result<Vec<(Agent, Option<PathBuf>)>, Answer> {
    let settings = settings(native).ok_or_else(no_settings)?;
    let agents = native.agents();
    let mut commands = HashMap::with_capacity(agents.len());
    for agent in agents.iter() {
        let command = command_of(&settings, agent).ok_or_else(|| unreadable_command(agent))?;
        commands.insert(agent.id().to_owned(), command);
    }
    let env = native.env.get();
    Ok(detect::locate(
        &agents,
        |agent| {
            commands
                .remove(agent.id())
                .unwrap_or_else(|| agent.default_command())
        },
        path_of(&env),
    ))
}

fn detect_installed(native: &Native, params: &Value) -> Answer {
    match found(native, params) {
        Ok(found) => Answer::Result(Value::Object(
            found
                .into_iter()
                .map(|(agent, at)| (agent.id().to_owned(), Value::Bool(at.is_some())))
                .collect(),
        )),
        Err(answer) => answer,
    }
}

/// Each agent's resolved program, or null, and whether the login shell has answered.
fn locate(native: &Native, params: &Value) -> Answer {
    match found(native, params) {
        Ok(found) => {
            let agents: Map<String, Value> = found
                .into_iter()
                .map(|(agent, at)| {
                    let at = at.map_or(Value::Null, |p| {
                        Value::String(p.to_string_lossy().into_owned())
                    });
                    (agent.id().to_owned(), at)
                })
                .collect();
            Answer::Result(json!({ "agents": agents, "shellResolved": native.env.resolved() }))
        }
        Err(answer) => answer,
    }
}

/// `env.PATH ?? env.Path`.
fn path_of(env: &env::Env) -> Option<&str> {
    let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    get("PATH").or_else(|| get("Path"))
}

/// Whether a flag a request carries is set: not null, false, 0 or empty.
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
        // Not a string: refused.
        _ => return bad_params("agent:listModels"),
    };
    let agents = native.agents();
    let request = ModelRequest {
        agent: request
            .get("agentType")
            .and_then(Value::as_str)
            .and_then(|id| agents.get(id))
            .cloned(),
        project_path,
        remote: truthy(request.get("remoteHostId")),
        refresh: truthy(request.get("refresh")),
    };
    // Answered without settings when there is nothing to ask.
    let askable = request.agent.as_ref().is_some_and(Agent::selects_models)
        && !request.remote
        && !request.project_path.is_empty()
        && !request.project_path.contains("{{");
    // An agent whose CLI cannot list its models takes a typed id.
    if askable && request.agent.as_ref().is_some_and(|a| !lists_models(a)) {
        return Answer::Result(
            CatalogAnswer::Unavailable {
                error: "This agent does not list its models; type a model id.".to_owned(),
            }
            .to_json(),
        );
    }
    let (agent, command, env) = match request.agent.clone() {
        Some(agent) if askable => {
            if !Path::new(&request.project_path).is_absolute() {
                return bad_params("agent:listModels");
            }
            let Some(settings) = settings(native) else {
                return no_settings();
            };
            let Some(command) = command_of(&settings, &agent) else {
                return unreadable_command(&agent);
            };
            let data_dir = native.db.get().and_then(|db| db.parent());
            let env = native.env.launch(&settings.env_passthrough, data_dir);
            (agent, command, env)
        }
        // Never asked: the catalog answers these unavailable before it looks
        // at the command.
        _ => {
            let answer = native
                .catalog
                .list(&request, &unused(), now_ms, || Err(DiscoveryError::Failed));
            return Answer::Result(answer.to_json());
        }
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
                &agent,
            )
        }
    };
    let answer = native.catalog.list(&request, &command, now_ms, discover);
    Answer::Result(answer.to_json())
}

/// Whether `agent`'s adapter says how to ask its CLI for models.
fn lists_models(agent: &Agent) -> bool {
    agent
        .spec()
        .models
        .as_ref()
        .is_some_and(|m| m.list.is_some())
}

/// A command for a request the catalog answers without asking.
fn unused() -> AgentCommand {
    AgentCommand {
        command: String::new(),
        args: Vec::new(),
        headless_args: None,
    }
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
        // Not a path: refused.
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
    let list = history::recent_sessions(
        &native.agents(),
        &homes,
        scope.as_ref(),
        history::DEFAULT_LIMIT,
    );
    Answer::Result(Value::Array(with_running(&list, |id| {
        super::sessions::running_terminal(native, id)
    })))
}

/// The recent sessions as JSON, each with `runningIn`, the terminal writing
/// it, when `running` names one: a conversation a terminal is writing is
/// running, not past, and resuming it shows that terminal.
fn with_running(
    list: &[history::RecentSession],
    running: impl Fn(&str) -> Option<String>,
) -> Vec<Value> {
    list.iter()
        .map(|r| {
            let mut json = r.to_json();
            if let Some(id) = running(&r.session_id) {
                json["runningIn"] = Value::String(id);
            }
            json
        })
        .collect()
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
    fn a_conversation_a_terminal_writes_is_marked_running() {
        let recent = |id: &str| history::RecentSession {
            session_id: id.to_owned(),
            agent: crate::adapters::testing::adapters()
                .agents()
                .get("acme")
                .unwrap()
                .clone(),
            display: id.to_owned(),
            project_path: "/p".into(),
            timestamp: 1.0,
            activity_count: 1.0,
        };
        let listed = with_running(&[recent("live"), recent("past")], |id| {
            (id == "live").then(|| "term-1".to_owned())
        });
        assert_eq!(listed[0]["runningIn"], "term-1");
        assert!(listed[1].get("runningIn").is_none());
    }

    #[test]
    fn reads_a_stored_command_and_defaults_the_rest() {
        let s = settings_with(json!({
            "gamma": { "command": "gam", "args": ["-x"], "headlessArgs": ["-p"] },
            "beta": { "command": "beta", "args": "not a list" }
        }));
        let agents = crate::adapters::testing::adapters().agents();
        let gamma = command_of(&s, agents.get("gamma").unwrap()).unwrap();
        assert_eq!(gamma.command, "gam");
        assert_eq!(gamma.args, ["-x"]);
        assert_eq!(gamma.headless_args.as_deref(), Some(&["-p".to_owned()][..]));
        let acme = agents.get("acme").unwrap();
        assert_eq!(command_of(&s, acme), Some(acme.default_command()));
        assert_eq!(command_of(&s, agents.get("beta").unwrap()), None);
    }

    #[test]
    fn answers_what_needs_no_settings_and_refuses_the_rest() {
        let native = Native::new();
        let root = if cfg!(windows) { "C:\\p" } else { "/p" };
        // Nothing to ask: answered whatever the database.
        for agent in ["gamma", "beta", "shell"] {
            let answer = native.call(
                "agent:listModels",
                &json!({ "agentType": agent, "projectPath": root }),
            );
            assert!(
                matches!(answer, Answer::Result(ref v) if v["status"] == "unavailable"),
                "{answer:?}"
            );
        }
        let Answer::Result(listed) = native.call("agent:list", &json!(null)) else {
            panic!("agent:list answers");
        };
        assert_eq!(listed[0]["id"], "acme");
        assert_eq!(listed[0]["models"], true);
        // To ask, the configured command is needed, and there is no database.
        assert_eq!(
            native.call(
                "agent:listModels",
                &json!({ "agentType": "acme", "projectPath": root })
            ),
            no_settings()
        );
        let bad = bad_params("agent:listModels");
        assert_eq!(
            native.call(
                "agent:listModels",
                &json!({ "agentType": "acme", "projectPath": "rel" })
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

//! The headless calls vornd answers itself with the Native server switch on:
//! `headless:create` and `headless:kill`.
//!
//! A headless agent runs on pipes in vornd's session holder, never on a
//! terminal: its prompt goes in on stdin, which then closes, and some agents
//! behave differently on a TTY. Each call answers as the server's
//! `HeadlessManager` does, from the copy of the session registry vornd
//! keeps ([`crate::registry`]), which then holds the record and tells the
//! server so (`native: true`): the record it created, its program's start,
//! or why it could not start. How the program ended the registry reads from
//! the session's exit effect ([`crate::registry::Registry::headless_exit`]),
//! whoever started it. The server follows the notes as it follows its own
//! starts: it reads the output, tells clients and the workflow waiting on
//! the agent, and lets the record go a while after the exit.
//!
//! A create prepares the workspace as a terminal's does
//! ([`super::sessions::workspace`]) and builds the process as
//! `buildHeadlessSpawnArgs` does ([`vorn_agents::launch::headless_spawn`]):
//! the arguments first, so arguments that cannot be built create nothing.
//! Nothing new starts while the server drains. A stop sends the program
//! `SIGTERM`, and `SIGKILL` [`FORCE_KILL_DELAY`] later if it still runs.
//!
//! The server keeps a create vornd cannot answer as it would: one for a
//! remote host, one whose params are not the shape its handler reads, and
//! every call while the registry does not hold the server's records or the
//! session holder is not connected. In shadow mode nothing here changes
//! anything: a create is compared as what each side would start ([`plan`]),
//! a stop as what each would answer ([`foresee`]).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tracing::{info, warn};
use vorn_agents::launch::shell as launch_shell;
use vorn_agents::launch::{
    display_name_from_prompt, headless_spawn, LaunchRequest, Machine, Platform, Quoting,
};
use vorn_agents::AgentCommand;
use vorn_git::repo::extract_worktree_name;
use vorn_sessiond_wire::{Io, Sig, SpawnSpec, Stdin};
use vorn_store::AgentSettings;

use super::sessions::{
    closing, fed_and_held, given, new_id, now_ms, plan_of, var, workspace, CreateRequest, Holds,
    Input,
};
use super::{agent, Answer, Native};
use crate::applink::{Closing, DRAINING_MESSAGE};
use crate::registry::{HeadlessSession, HeadlessStatus};

/// How long after `SIGTERM` an agent that still runs is sent `SIGKILL`.
pub const FORCE_KILL_DELAY: Duration = Duration::from_secs(5);

/// The longest a display name taken from the prompt runs.
const PROMPT_NAME_LEN: usize = 60;

/// Answers `method` with `params`.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    match method {
        "headless:create" => match CreateRequest::read(params) {
            // A remote host's agent is the server's.
            Some(req) if req.remote().is_none() => create(native, &req),
            _ => Answer::Forward,
        },
        "headless:kill" => match params.as_str() {
            Some(id) => kill(native, id),
            None => Answer::Forward,
        },
        _ => Answer::Forward,
    }
}

/// Whether [`foresee`] can say what `method` would answer.
pub fn foresees(method: &str) -> bool {
    method == "headless:kill"
}

/// What vornd would answer `headless:kill`, read from the copy without
/// changing it: nothing, as the server answers it, for an agent the copy
/// holds. `None` when the answer is the server's own.
pub fn foresee(native: &Native, params: &Value) -> Option<Answer> {
    let id = params.as_str()?;
    native
        .registry
        .get()?
        .read(|r| r.headless_record(id).map(|_| Answer::Void))?
}

/// Whether vornd can start an agent now: it starts headless agents (as
/// `vornd:hello` told the server, which follows them only then), and the
/// registry and the session holder are ready ([`fed_and_held`]).
fn can_start(native: &Native) -> bool {
    let creates = native.link.get().is_some_and(|l| l.creates_headless());
    creates && fed_and_held(native)
}

/// The server's refusal of a new agent while it drains (`isDraining`), if
/// it does. A handover does not refuse one: the server's handler does not ask.
fn refusal(native: &Native) -> Option<&'static str> {
    (closing(native)? == Closing::Draining).then_some(DRAINING_MESSAGE)
}

/// `headless:create` for a local agent.
fn create(native: &Native, req: &CreateRequest) -> Answer {
    if !can_start(native) {
        return Answer::Forward;
    }
    if let Some(why) = refusal(native) {
        return Answer::Error(why.to_owned());
    }
    let Some(settings) = agent::settings(native) else {
        return Answer::Forward;
    };
    let Some(config) = agent::command_of(&settings, req.agent) else {
        return Answer::Forward;
    };
    let mut holds = Holds::new(native);
    // Held from here until the record is in, so a worktree action in between
    // sees it as in use: the worktree it names, and one it creates.
    if let Some(existing) = given(req.existing_worktree_path.as_deref()) {
        holds.hold(existing);
    }
    let launch = match prepare(native, req, &settings, &config, &mut holds) {
        Ok(launch) => launch,
        Err(answer) => return answer,
    };
    // Again: draining may have begun while the workspace was prepared.
    if let Some(why) = refusal(native) {
        return Answer::Error(why.to_owned());
    }
    let answer = register(native, launch);
    drop(holds);
    answer
}

/// Everything a headless agent starts with (`createHeadless`, up to the
/// spawn): its record, where and how it runs, and its prompt.
#[derive(Debug)]
struct Launch {
    record: HeadlessSession,
    cwd: String,
    argv: Vec<String>,
    env: Vec<(String, String)>,
    /// The prompt, for the program's stdin.
    stdin: Option<String>,
}

/// Works out the launch for `req`: the process first, so arguments that
/// cannot be built create nothing, then the workspace.
fn prepare(
    native: &Native,
    req: &CreateRequest,
    settings: &AgentSettings,
    config: &AgentCommand,
    holds: &mut Holds<'_>,
) -> Result<Launch, Answer> {
    let resume = given(req.resume_session_id.as_deref());
    // Pinned before the arguments are built, so a fresh launch carries the
    // id it will resume by.
    let mut agent_session_id = match req.agent {
        vorn_agents::Agent::Codex => req.resume_session_id.clone(),
        _ => None,
    };
    let mut session_id = req.session_id.clone();
    if req.agent.pins_session_ids() {
        agent_session_id = Some(resume.map_or_else(
            || {
                let minted = new_id();
                session_id = Some(minted.clone());
                minted
            },
            str::to_owned,
        ));
    }
    let launch = LaunchRequest {
        agent: req.agent,
        args: req.args.clone(),
        model: req.model.clone(),
        remote_host_id: None,
        resume_session_id: req.resume_session_id.clone(),
        session_id,
        initial_prompt: req.initial_prompt.clone(),
    };
    let data_dir = native.db.get().and_then(|db| db.parent());
    let env = native.env.launch(&settings.env_passthrough, data_dir);
    let default_shell = launch_shell::default_shell(None, Platform::HOST, var);
    let machine = Machine {
        platform: Platform::HOST,
        quoting: Quoting::local(Platform::HOST, &default_shell),
    };
    let spawn = headless_spawn(&launch, Some(config), &env, &machine)
        .map_err(|e| Answer::Error(e.to_string()))?;
    let (argv, launch_command) = process_of(&spawn.command, &spawn.args);

    let space = workspace(native, req, holds)?;
    let existing = given(req.existing_worktree_path.as_deref());
    // The worktree named, there or not, else the one made for the call.
    let worktree_path = existing.map(str::to_owned).or_else(|| {
        space
            .made
            .as_ref()
            .filter(|_| req.use_worktree)
            .map(|(path, _)| path.clone())
    });
    let worktree_name = match (&space.reused, &space.made) {
        (Some(reused), _) => Some(
            given(req.worktree_name.as_deref())
                .map_or_else(|| extract_worktree_name(reused), str::to_owned),
        ),
        (None, Some((_, name))) => Some(name.clone()),
        (None, None) => None,
    };
    let record = HeadlessSession {
        id: new_id(),
        pid: 0,
        agent_type: req.agent.id().to_owned(),
        project_name: req.project_name.clone(),
        project_path: req.project_path.clone(),
        display_name: given(req.display_name.as_deref())
            .map(str::to_owned)
            .or_else(|| {
                given(req.initial_prompt.as_deref())
                    .and_then(|p| display_name_from_prompt(p, PROMPT_NAME_LEN))
            }),
        branch: space.branch.filter(|b| !b.is_empty()),
        is_worktree: Some(worktree_path.is_some()),
        worktree_path,
        worktree_name,
        status: HeadlessStatus::Running,
        exit_code: None,
        started_at: now_ms(),
        ended_at: None,
        workflow_id: req.workflow_id.clone(),
        workflow_name: req.workflow_name.clone(),
        agent_session_id: agent_session_id.filter(|a| !a.is_empty()),
        launch_command: Some(launch_command),
        rev: None,
        exit_at: None,
        other: serde_json::Map::new(),
    };
    Ok(Launch {
        record,
        cwd: space.cwd,
        argv,
        env,
        stdin: spawn.stdin,
    })
}

/// The process to start and the line that says what it is. On Windows the
/// program runs through cmd.exe, as the server's `shell: true` runs it,
/// with each argument quoted for cmd so none is word-split; on POSIX the
/// arguments reach it as they are.
fn process_of(command: &str, args: &[String]) -> (Vec<String>, String) {
    if cfg!(windows) {
        let quoted: Vec<String> = std::iter::once(command)
            .chain(args.iter().map(String::as_str))
            .map(|a| Quoting::Cmd.quote(a).into_owned())
            .collect();
        let line = quoted.join(" ");
        let comspec = var("ComSpec").unwrap_or_else(|| "cmd.exe".to_owned());
        let argv = vec![
            comspec,
            "/d".into(),
            "/s".into(),
            "/c".into(),
            format!("\"{line}\""),
        ];
        (argv, line)
    } else {
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push(command.to_owned());
        argv.extend(args.iter().cloned());
        (argv.clone(), argv.join(" "))
    }
}

/// Puts the record in the registry and starts the program. The answer is
/// the record, as the server answers before its program is up.
fn register(native: &Native, launch: Launch) -> Answer {
    let (Some(registry), Some(host)) = (native.registry.get(), native.host.get()) else {
        return Answer::Forward;
    };
    let Launch {
        record,
        cwd,
        argv,
        env,
        stdin,
    } = launch;
    let id = record.id.clone();
    let answer = serde_json::to_value(&record).unwrap_or(Value::Null);
    let made = registry.change(|r| match r.create_headless(record) {
        Ok(notes) => (Ok(()), notes),
        Err(e) => (Err(e), Vec::new()),
    });
    match made {
        Some(Ok(())) => {}
        Some(Err(e)) => return Answer::Error(e.to_string()),
        None => return Answer::Error("The Vorn server is not connected to vornd".to_owned()),
    }
    info!(
        %id, %cwd, line = %argv.join(" "),
        prompt = stdin.as_ref().map_or(0, String::len),
        "launching a headless agent"
    );
    let spec = SpawnSpec {
        argv,
        cwd,
        env,
        io: Io::Piped { stdin: Stdin::Pipe },
        ring_bytes: None,
    };
    native.sessions.lock_starting().insert(id.clone(), None);
    let (registry, host_after) = (Arc::clone(registry), Arc::clone(host));
    let sessions = Arc::clone(&native.sessions);
    let name = id.clone();
    host.start(
        spec,
        name,
        Input::Prompt(stdin.map(String::into_bytes)),
        Box::new(move |outcome| {
            let pending = sessions.lock_starting().remove(&id).flatten();
            match outcome {
                Ok(s) => {
                    registry.change(|r| {
                        (
                            (),
                            r.headless_started(&id, s.pid, s.epoch)
                                .into_iter()
                                .collect(),
                        )
                    });
                    // Stopped while it started: told now that it is up.
                    if let Some(sig) = pending {
                        stop(host_after.as_ref(), &id, sig);
                    }
                }
                Err(why) => {
                    warn!(%id, %why, "vornd could not start this headless agent");
                    registry.change(|r| ((), r.headless_failed(&id, &why).into_iter().collect()));
                }
            }
        }),
    );
    Answer::Result(answer)
}

/// Sends `sig`, and `SIGKILL` after [`FORCE_KILL_DELAY`] when it was `SIGTERM`.
fn stop(host: &dyn super::sessions::Host, id: &str, sig: Sig) {
    host.signal(id, sig);
    if sig == Sig::Term {
        host.signal_after(id, Sig::Kill, FORCE_KILL_DELAY);
    }
}

/// `headless:kill`: the agent is asked to stop, and made to a while later.
/// Answers nothing, as the server does, for an agent the copy holds, ended
/// or not; one it does not hold is the server's.
fn kill(native: &Native, id: &str) -> Answer {
    let running = native.registry.get().and_then(|r| {
        r.read(|r| {
            r.headless_record(id)
                .map(|h| h.status == HeadlessStatus::Running)
        })
    });
    let Some(Some(running)) = running else {
        return Answer::Forward;
    };
    if !running || native.sessions.doom(id, Sig::Term) {
        return Answer::Void;
    }
    if let Some(host) = native.host.get() {
        stop(host.as_ref(), id, Sig::Term);
    }
    Answer::Void
}

/// What a create would start, worked out without starting or changing
/// anything, for the comparison with the spawn the server asks for
/// ([`super::sessions::plan`]). `None` when it cannot be worked out without
/// a change (a worktree to make, a branch to check out).
pub fn plan(native: &Native, req: &CreateRequest) -> Option<Value> {
    if req.remote().is_some() {
        return None;
    }
    let settings = agent::settings(native)?;
    let config = agent::command_of(&settings, req.agent)?;
    let existing = given(req.existing_worktree_path.as_deref());
    let reuses = existing.is_some_and(|e| Path::new(e).exists());
    if !reuses && given(req.branch.as_deref()).is_some() {
        return None;
    }
    let mut holds = Holds::new(native);
    let launch = prepare(native, req, &settings, &config, &mut holds).ok()?;
    let keys = launch.env.into_iter().map(|(k, _)| k).collect();
    let record = serde_json::to_value(&launch.record).unwrap_or(Value::Null);
    Some(plan_of(launch.argv, &launch.cwd, keys, &record))
}

#[cfg(test)]
mod tests {
    use super::super::sessions::plan_record;
    use super::super::sessions::tests::{fed, Fed};
    use super::*;
    use serde_json::json;
    use vorn_agents::Agent;

    /// A project path that is absolute where the test runs.
    fn project() -> &'static str {
        if cfg!(windows) {
            "C:\\p"
        } else {
            "/p"
        }
    }

    /// vornd starting headless agents, with the agents' default commands.
    fn ready() -> Fed {
        let fed = fed();
        fed.link.set_creates_headless();
        fed.native.set_database(
            std::env::temp_dir()
                .join("vornd-no-such-db")
                .join("vorn.db"),
        );
        fed
    }

    fn request(agent: &str, extra: Value) -> Value {
        let mut req = json!({
            "agentType": agent, "projectName": "p", "projectPath": project(),
            "initialPrompt": "write the tests\nall of them", "headless": true,
        });
        if let (Value::Object(req), Value::Object(extra)) = (&mut req, extra) {
            req.extend(extra);
        }
        req
    }

    #[test]
    fn starts_every_agent_on_pipes_with_its_prompt_on_stdin() {
        let fed = ready();
        for agent in Agent::ALL {
            let params = request(
                agent.id(),
                json!({ "workflowId": "wf", "workflowName": "w" }),
            );
            let Answer::Result(record) = call(&fed.native, "headless:create", &params) else {
                panic!("{agent:?} was not started");
            };
            assert_eq!(record["agentType"], agent.id());
            assert_eq!(record["status"], "running");
            assert_eq!(record["pid"], 0);
            assert_eq!(record["displayName"], "write the tests all of them");
            assert_eq!(record["isWorktree"], false);
            assert_eq!(
                (&record["workflowId"], &record["workflowName"]),
                (&json!("wf"), &json!("w"))
            );
            assert_eq!(
                record["agentSessionId"].is_string(),
                agent.pins_session_ids()
            );
            let (spec, input) = fed.host.last_start();
            assert_eq!(spec.io, Io::Piped { stdin: Stdin::Pipe });
            assert_eq!(spec.cwd, project());
            assert_eq!(
                input,
                Input::Prompt(Some(b"write the tests\nall of them".to_vec()))
            );
            // The prompt is never on the command line.
            assert!(!spec.argv.iter().any(|a| a.contains("write the tests")));
            if !cfg!(windows) {
                assert_eq!(record["launchCommand"], spec.argv.join(" "));
            }
            let id = record["id"].as_str().unwrap().to_owned();
            assert_eq!(
                fed.registry
                    .read(|r| r.headless_record(&id).map(|h| h.launch_command.clone()))
                    .flatten()
                    .flatten(),
                record["launchCommand"].as_str().map(str::to_owned)
            );
        }
        assert_eq!(
            fed.registry.read(|r| r.headless().count()),
            Some(Agent::ALL.len())
        );
    }

    #[test]
    fn an_agent_without_a_prompt_has_its_stdin_closed_at_once() {
        let fed = ready();
        let params = json!({ "agentType": "claude", "projectName": "p", "projectPath": project() });
        let Answer::Result(record) = call(&fed.native, "headless:create", &params) else {
            panic!("not started");
        };
        assert!(record.get("displayName").is_none());
        let (spec, input) = fed.host.last_start();
        assert_eq!(input, Input::Prompt(None));
        assert_eq!(spec.argv.last().map(String::as_str), Some(""));
    }

    #[test]
    fn tells_the_start_and_the_failure_as_native_notes() {
        let fed = ready();
        let mut notes = fed.registry.subscribe();
        let Answer::Result(record) =
            call(&fed.native, "headless:create", &request("codex", json!({})))
        else {
            panic!("not started");
        };
        let id = record["id"].as_str().unwrap().to_owned();
        let created = notes.try_recv().unwrap();
        assert_eq!(
            (&created["native"], &created["created"], &created["kind"]),
            (&json!(true), &json!(true), &json!("headless"))
        );
        fed.host.up(77);
        let started = notes.try_recv().unwrap();
        assert_eq!(started["started"], json!({ "pid": 77, "epoch": 1 }));
        assert_eq!(started["record"]["pid"], 77);
        assert_eq!(
            fed.registry.read(|r| r.headless_record(&id).map(|h| h.pid)),
            Some(Some(77))
        );

        call(
            &fed.native,
            "headless:create",
            &request("gemini", json!({})),
        );
        notes.try_recv().unwrap();
        fed.host.down("no such program");
        let failed = notes.try_recv().unwrap();
        assert_eq!(failed["failed"], "no such program");
        assert_eq!(failed["record"]["status"], "exited");
        assert_eq!(failed["record"]["exitCode"], 1);
    }

    #[test]
    fn stops_an_agent_with_term_then_kill_and_one_still_starting_once_it_is_up() {
        let fed = ready();
        let Answer::Result(record) = call(
            &fed.native,
            "headless:create",
            &request("claude", json!({})),
        ) else {
            panic!("not started");
        };
        let id = record["id"].as_str().unwrap().to_owned();
        // Not up yet: nothing to signal until it is.
        assert_eq!(call(&fed.native, "headless:kill", &json!(id)), Answer::Void);
        assert!(fed.host.signalled().is_empty());
        fed.host.up(5);
        assert_eq!(
            fed.host.signals(),
            [(id.clone(), Sig::Term), (id.clone(), Sig::Kill)]
        );
        // Up: signalled at once.
        assert_eq!(call(&fed.native, "headless:kill", &json!(id)), Answer::Void);
        assert_eq!(fed.host.signals().len(), 4);
        // Ended: nothing to stop, as the server answers.
        fed.registry.headless_exit(
            &id,
            3,
            crate::registry::Stamp {
                epoch: 1,
                rseq: 4,
                index: 0,
            },
        );
        assert_eq!(call(&fed.native, "headless:kill", &json!(id)), Answer::Void);
        assert_eq!(fed.host.signals().len(), 4);
        // One the copy does not hold is the server's.
        assert_eq!(
            call(&fed.native, "headless:kill", &json!("x")),
            Answer::Forward
        );
        assert_eq!(
            call(&fed.native, "headless:kill", &json!(3)),
            Answer::Forward
        );
    }

    #[test]
    fn creates_nothing_unless_vornd_starts_headless_agents() {
        let fed = fed();
        assert_eq!(
            call(
                &fed.native,
                "headless:create",
                &request("claude", json!({}))
            ),
            Answer::Forward
        );
        // A remote host's agent is the server's.
        let fed = ready();
        assert_eq!(
            call(
                &fed.native,
                "headless:create",
                &request("claude", json!({ "remoteHostId": "h" }))
            ),
            Answer::Forward
        );
        assert!(fed.host.starts.lock().unwrap().is_empty());
    }

    #[test]
    fn refuses_a_new_agent_while_the_server_drains_and_not_while_it_hands_over() {
        let fed = ready();
        fed.link.set_closing(Closing::Draining);
        assert_eq!(
            call(
                &fed.native,
                "headless:create",
                &request("claude", json!({}))
            ),
            Answer::Error(DRAINING_MESSAGE.to_owned())
        );
        fed.link.set_closing(Closing::HandingOver);
        assert!(matches!(
            call(
                &fed.native,
                "headless:create",
                &request("claude", json!({}))
            ),
            Answer::Result(_)
        ));
    }

    #[test]
    fn plans_a_create_and_foresees_a_stop_without_starting_anything() {
        let fed = ready();
        let req = CreateRequest::read(&request("opencode", json!({}))).unwrap();
        let planned = plan(&fed.native, &req).unwrap();
        assert_eq!(planned["cwd"], project());
        assert!(planned["envKeys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k == "PATH"));
        assert_eq!(planned["record"]["agentType"], "opencode");
        assert!(planned["record"].get("startedAt").is_none());
        let line: Vec<&str> = planned["argv"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        if !cfg!(windows) {
            assert_eq!(planned["record"]["launchCommand"], line.join(" "));
        }
        assert_eq!(planned["record"], plan_record(&planned["record"]));
        assert!(fed.host.starts.lock().unwrap().is_empty());
        assert_eq!(fed.registry.read(|r| r.headless().count()), Some(0));
        // A worktree to make is a change: not planned.
        let branched = CreateRequest::read(&request(
            "opencode",
            json!({ "useWorktree": true, "branch": "b" }),
        ))
        .unwrap();
        assert_eq!(plan(&fed.native, &branched), None);

        assert_eq!(foresee(&fed.native, &json!("x")), None);
        call(
            &fed.native,
            "headless:create",
            &request("claude", json!({})),
        );
        let id = fed
            .registry
            .read(|r| r.headless().next().unwrap().id.clone())
            .unwrap();
        assert_eq!(foresee(&fed.native, &json!(id)), Some(Answer::Void));
        assert!(fed.host.signalled().is_empty());
    }
}

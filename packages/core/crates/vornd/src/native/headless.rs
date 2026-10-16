//! The headless calls vornd answers itself:
//! `headless:create` and `headless:kill`.
//!
//! A headless agent runs on pipes in vornd's session holder, never on a
//! terminal: its prompt goes in on stdin, which then closes, and some agents
//! behave differently on a TTY. Each call answers as the server's
//! `HeadlessManager` did, from the session registry vornd keeps
//! ([`crate::registry`]), which then holds the record: the record it
//! created, its program's start, or why it could not start. How the program
//! ended the registry reads from the session's exit effect
//! ([`crate::registry::Registry::headless_exit`]). Clients are sent the
//! output as it comes (`headless:data`) and the end
//! ([`super::session_events`]), and the record goes a while after the exit.
//!
//! A create prepares the workspace as a terminal's does
//! ([`super::sessions::workspace`]) and builds the process as
//! `buildHeadlessSpawnArgs` does ([`vorn_agents::launch::headless_spawn`]):
//! the arguments first, so arguments that cannot be built create nothing.
//! A stop sends the program
//! `SIGTERM`, and `SIGKILL` [`FORCE_KILL_DELAY`] later if it still runs.
//!
//! A create naming a remote host runs the agent here, as the server ran it.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tracing::{info, warn};
use vorn_agents::launch::shell as launch_shell;
use vorn_agents::launch::{
    display_name_from_prompt, headless_spawn, LaunchRequest, Machine, Platform, Quoting,
};
use vorn_agents::AgentCommand;
use vorn_git::repo::extract_worktree_name;
use vorn_sessiond_wire::{Io, Sig, SpawnSpec, Stdin};
use vorn_store::AgentSettings;

use super::sessions::NO_HOLDER;
use super::sessions::{
    fed_and_held, given, new_id, now_ms, var, workspace, CreateRequest, Holds, Input,
};
use super::{agent, bad_params, not_answered, not_ready, Answer, Native};
use crate::registry::{HeadlessSession, HeadlessStatus};

/// How long after `SIGTERM` an agent that still runs is sent `SIGKILL`.
pub const FORCE_KILL_DELAY: Duration = Duration::from_secs(5);

/// The longest a display name taken from the prompt runs.
const PROMPT_NAME_LEN: usize = 60;

/// Answers `method` with `params`.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    match method {
        // A remote host named runs the agent here, as the server ran it.
        "headless:create" => match CreateRequest::read(params, &native.agents()) {
            Some(req) => create(
                native,
                &CreateRequest {
                    remote_host_id: None,
                    ..req
                },
            ),
            None => bad_params(method),
        },
        "headless:kill" => match params.as_str() {
            Some(id) => kill(native, id),
            None => bad_params(method),
        },
        _ => not_answered(method),
    }
}

/// Why vornd cannot start an agent now: the session holder is not connected.
fn unstartable(native: &Native) -> Option<Answer> {
    (!fed_and_held(native)).then(|| Answer::Error(NO_HOLDER.into()))
}

/// `headless:create` for a local agent.
fn create(native: &Native, req: &CreateRequest) -> Answer {
    if let Some(answer) = unstartable(native) {
        return answer;
    }
    let Some(settings) = agent::settings(native) else {
        return agent::no_settings();
    };
    let Some(config) = agent::command_of(&settings, &req.agent) else {
        return agent::unreadable_command(&req.agent);
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
    // An agent resumed headless by a subcommand runs on the conversation it names.
    let resumes_by_tail = req
        .agent
        .spec()
        .headless
        .as_ref()
        .is_some_and(|h| !h.resume_tail.is_empty());
    let mut agent_session_id = resumes_by_tail
        .then(|| req.resume_session_id.clone())
        .flatten();
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
        agent: req.agent.clone(),
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
        origin: crate::registry::SessionOrigin(req.origin.clone()),
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
        return not_ready();
    };
    let Launch {
        record,
        cwd,
        argv,
        env,
        stdin,
    } = launch;
    let id = record.id.clone();
    let agent_type = record.agent_type.clone();
    let answer = serde_json::to_value(&record).unwrap_or(Value::Null);
    let created = json!({
        "agentType": record.agent_type,
        "projectName": record.project_name,
        "projectPath": record.project_path,
        "headless": true,
    });
    let made = registry.change(|r| match r.create_headless(record) {
        Ok(notes) => (Ok(()), notes),
        Err(e) => (Err(e), Vec::new()),
    });
    match made {
        Some(Ok(())) => super::session_log::log_event(native, &id, "created", Some(created)),
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
    let (output_tx, output) = tokio::sync::mpsc::unbounded_channel();
    let (ended_tx, _ended) = tokio::sync::oneshot::channel();
    let watch = super::sessions::Watch {
        output: output_tx,
        ended: ended_tx,
    };
    let reader = stream_reader(native, &agent_type, &id);
    if let (Some(notifier), Ok(runtime)) =
        (native.notifier(), tokio::runtime::Handle::try_current())
    {
        runtime.spawn(tell_output(notifier, id.clone(), output, reader));
    }
    host.start_watched(
        spec,
        name,
        Input::Prompt(stdin.map(String::into_bytes)),
        watch,
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

/// Tells clients what a headless agent prints, as text, as it prints it (`headless:data`).
async fn tell_output(
    notifier: super::Notifier,
    id: String,
    mut output: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    reader: Option<tokio::sync::mpsc::Sender<String>>,
) {
    let mut text = super::script::Utf8::default();
    let mut lines = String::new();
    while let Some(bytes) = output.recv().await {
        let data = text.push(&bytes);
        if data.is_empty() {
            continue;
        }
        if let Some(reader) = &reader {
            lines.push_str(&data);
            while let Some(end) = lines.find('\n') {
                let line: String = lines.drain(..=end).collect();
                // A reader that falls behind loses lines rather than holding the output up.
                let _ = reader.try_send(line.trim_end_matches(['\r', '\n']).to_owned());
            }
        }
        notifier.tell(
            "headless:data",
            json!({ "id": id, "data": data }),
            Some(&id),
        );
    }
}

/// How many lines a headless run's reader holds before it drops new ones.
const STREAM_BACKLOG: usize = 1024;
/// The most lines one `agent.stream` call carries.
const STREAM_BATCH: usize = 256;

/// The reader of headless session `id`'s output, when its agent's adapter
/// reads its stream with a program (`[agent.headless] stream`).
fn stream_reader(
    native: &Native,
    agent: &str,
    id: &str,
) -> Option<tokio::sync::mpsc::Sender<String>> {
    let adapters = native.adapters();
    let streams = adapters
        .get(agent)?
        .spec()
        .headless
        .as_ref()
        .is_some_and(|h| h.stream);
    if !streams {
        return None;
    }
    let around = Around {
        extension: adapters.extension_of(agent)?,
        extensions: Arc::clone(native.extensions()?),
        registry: Arc::clone(native.registry.get()?),
        events: Arc::clone(native.events()),
    };
    let runtime = tokio::runtime::Handle::try_current().ok()?;
    let (tx, rx) = tokio::sync::mpsc::channel(STREAM_BACKLOG);
    runtime.spawn(read_stream(around, id.to_owned(), rx));
    Some(tx)
}

/// What a stream's reader reaches: the program that reads it, and where
/// what it read goes.
struct Around {
    extension: String,
    extensions: Arc<super::extensions::Extensions>,
    registry: Arc<crate::registry::SessionRegistry>,
    events: Arc<vorn_extensions::events::Bus>,
}

/// Hands what a headless run prints to its adapter's program, a batch of
/// lines at a time, and takes in what it reads there: the conversation the
/// run took, and hook events as the agent's hooks would post them.
async fn read_stream(around: Around, id: String, mut rx: tokio::sync::mpsc::Receiver<String>) {
    use vorn_extensions::api::types::CallCtx;
    while let Some(first) = rx.recv().await {
        let mut lines = vec![first];
        while lines.len() < STREAM_BATCH {
            match rx.try_recv() {
                Ok(line) => lines.push(line),
                Err(_) => break,
            }
        }
        let ctx = CallCtx {
            caller: "vornd".to_owned(),
            project: None,
        };
        let args = json!({ "session": id, "lines": lines });
        let extension = &around.extension;
        match around
            .extensions
            .router()
            .call(extension, "agent.stream", args, ctx)
            .await
        {
            Ok(read) => took(&around.registry, &around.events, &id, &read),
            Err(err) => warn!(%extension, %err, "an adapter did not read a headless stream"),
        }
    }
}

/// Applies what an adapter's program read from session `id`'s stream.
fn took(
    registry: &crate::registry::SessionRegistry,
    events: &vorn_extensions::events::Bus,
    id: &str,
    read: &Value,
) {
    if let Some(conversation) = read
        .get("conversation")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
    {
        registry.headless_conversation(id, conversation);
    }
    for event in read
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = event.get("event").and_then(Value::as_str) else {
            continue;
        };
        let tool = event.get("tool").and_then(Value::as_str);
        events.emit(
            vorn_extensions::events::AGENT_HOOK,
            || json!({ "sessionId": id, "event": name, "tool": tool }),
        );
    }
}

/// Sends `sig`, and `SIGKILL` after [`FORCE_KILL_DELAY`] when it was `SIGTERM`.
fn stop(host: &dyn super::sessions::Host, id: &str, sig: Sig) {
    host.signal(id, sig);
    if sig == Sig::Term {
        host.signal_after(id, Sig::Kill, FORCE_KILL_DELAY);
    }
}

/// `headless:kill`: the agent is asked to stop, and made to a while later.
/// Answers nothing, as the server does, whether or not an agent runs under `id`.
fn kill(native: &Native, id: &str) -> Answer {
    let running = native.registry.get().and_then(|r| {
        r.read(|r| {
            r.headless_record(id)
                .map(|h| h.status == HeadlessStatus::Running)
        })
    });
    let Some(running) = running else {
        return not_ready();
    };
    // An agent no record names is answered with nothing, as `killHeadless` answers it.
    let Some(running) = running else {
        return Answer::Void;
    };
    if !running || native.sessions.doom(id, Sig::Term) {
        return Answer::Void;
    }
    if let Some(host) = native.host.get() {
        stop(host.as_ref(), id, Sig::Term);
    }
    Answer::Void
}

#[cfg(test)]
mod tests {
    use super::super::sessions::tests::{fed, Fed};
    use super::*;
    use serde_json::json;

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
        let agents = fed.native.agents();
        for agent in agents.iter() {
            let params = request(
                agent.id(),
                json!({ "origin": { "extension": "vorn.workflows", "id": "wf", "label": "w" } }),
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
                record["origin"],
                json!({ "extension": "vorn.workflows", "id": "wf", "label": "w" })
            );
            assert!(record.get("workflowId").is_none());
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
            Some(agents.len())
        );
    }

    #[test]
    fn an_agent_without_a_prompt_has_its_stdin_closed_at_once() {
        let fed = ready();
        let params = json!({ "agentType": "acme", "projectName": "p", "projectPath": project() });
        let Answer::Result(record) = call(&fed.native, "headless:create", &params) else {
            panic!("not started");
        };
        assert!(record.get("displayName").is_none());
        let (spec, input) = fed.host.last_start();
        assert_eq!(input, Input::Prompt(None));
        let last = spec.argv.last().map(String::as_str).unwrap_or_default();
        if cfg!(windows) {
            // The cmd.exe line, its empty prompt quoted.
            assert!(last.ends_with(r#" -p """"#), "{last}");
        } else {
            assert_eq!(last, "");
        }
    }

    #[tokio::test]
    async fn takes_the_conversation_and_hook_events_an_adapter_read() {
        use vorn_extensions::events::{Owner, Queue};
        let fed = ready();
        let Answer::Result(record) =
            call(&fed.native, "headless:create", &request("gamma", json!({})))
        else {
            panic!("not started");
        };
        let id = record["id"].as_str().unwrap().to_owned();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        fed.native.events().subscribe_with(
            Owner::Core("test".into()),
            &[vorn_extensions::events::AGENT_HOOK.to_owned()],
            Queue { tx, tally: None },
        );
        let read = json!({ "conversation": "conv-1", "events": [{ "event": "PreToolUse", "tool": "edit" }, { "nope": 1 }] });
        took(&fed.registry, fed.native.events(), &id, &read);
        assert_eq!(
            fed.registry
                .read(|r| r
                    .headless_record(&id)
                    .and_then(|h| h.agent_session_id.clone()))
                .flatten()
                .as_deref(),
            Some("conv-1")
        );
        let event = rx.recv().await.unwrap();
        assert_eq!(
            event.data,
            json!({ "sessionId": id, "event": "PreToolUse", "tool": "edit" })
        );
        assert!(rx.try_recv().is_err());
        // An adapter that does not read streams gets no reader.
        assert!(stream_reader(&fed.native, "gamma", &id).is_none());
    }

    #[test]
    fn tells_the_start_and_the_failure_as_native_notes() {
        let fed = ready();
        let mut notes = fed.registry.subscribe();
        let Answer::Result(record) =
            call(&fed.native, "headless:create", &request("beta", json!({})))
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

        call(&fed.native, "headless:create", &request("gamma", json!({})));
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
        let Answer::Result(record) =
            call(&fed.native, "headless:create", &request("acme", json!({})))
        else {
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
        // One no record names: nothing, as the server answers it.
        assert_eq!(
            call(&fed.native, "headless:kill", &json!("x")),
            Answer::Void
        );
        assert_eq!(
            call(&fed.native, "headless:kill", &json!(3)),
            bad_params("headless:kill")
        );
    }

    #[test]
    fn reads_a_create_and_runs_a_remote_hosts_agent_here() {
        let fed = ready();
        assert_eq!(
            call(&fed.native, "headless:create", &json!({})),
            bad_params("headless:create")
        );
        // A remote host named runs the agent here, as the server ran it.
        let made = call(
            &fed.native,
            "headless:create",
            // A remote project is a POSIX path whatever this host is.
            &request("acme", json!({ "remoteHostId": "h", "projectPath": "/p" })),
        );
        assert!(matches!(made, Answer::Result(_)), "{made:?}");
        assert_eq!(fed.host.starts.lock().unwrap().len(), 1);
    }
}

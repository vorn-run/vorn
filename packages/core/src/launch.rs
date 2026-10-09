//! The napi face of `vorn_agents::launch`, for tests only: the server builds
//! its launches in TypeScript, and these let `tests/launch-parity.test.ts`
//! put the same input through both and compare. No server code calls them.
//!
//! Each converts the server's shapes into the crate's and back; the crate
//! does the rest. All are cheap and synchronous: string work and, for the
//! command's PATH lookup and the shim check, a handful of `stat`s.

use std::collections::HashMap;

use napi_derive::napi;
use vorn_agents::launch::{self, shell, LaunchRequest, Machine, Platform, Quoting};
use vorn_agents::{Agent, AgentCommand};

/// The part of `CreateTerminalPayload` a launch reads. Other fields the
/// server sends are ignored.
#[napi(object)]
pub struct LaunchPayload {
    pub agent_type: String,
    pub args: Option<Vec<String>>,
    pub model: Option<String>,
    pub remote_host_id: Option<String>,
    pub resume_session_id: Option<String>,
    pub session_id: Option<String>,
    pub initial_prompt: Option<String>,
}

/// One agent's configured command (`AgentCommandConfig`).
#[napi(object)]
pub struct LaunchCommand {
    pub command: String,
    pub args: Vec<String>,
    pub headless_args: Option<Vec<String>>,
    pub fallback_command: Option<String>,
    pub fallback_args: Option<Vec<String>>,
}

/// The machine to work the launch out for, which the TypeScript reads from
/// its process.
#[napi(object)]
pub struct LaunchHost {
    /// `process.platform`.
    pub platform: String,
    /// The default shell, which decides quoting on Windows.
    pub shell: String,
}

/// `HeadlessSpawnArgs`.
#[napi(object)]
pub struct HeadlessArgs {
    pub command: String,
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

/// One word of a launch line: as written, and as the shell passes it on.
#[napi(object)]
pub struct LaunchToken {
    pub raw: String,
    pub value: String,
}

/// `ShellSetup`.
#[napi(object)]
pub struct ShellSetupResult {
    pub env: HashMap<String, String>,
    pub args: Option<Vec<String>>,
}

fn reason(message: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(message.to_string())
}

/// The request, or the server's error for a shell or an unknown agent.
fn request(payload: LaunchPayload, shell_message: &str) -> napi::Result<LaunchRequest> {
    let agent = match Agent::from_id(&payload.agent_type) {
        Some(agent) => agent,
        None if payload.agent_type == "shell" => return Err(reason(shell_message)),
        None => {
            return Err(reason(format!(
                "unknown agent type: {}",
                payload.agent_type
            )))
        }
    };
    Ok(LaunchRequest {
        agent,
        args: payload.args,
        model: payload.model,
        remote_host_id: payload.remote_host_id,
        resume_session_id: payload.resume_session_id,
        session_id: payload.session_id,
        initial_prompt: payload.initial_prompt,
    })
}

fn command(commands: &HashMap<String, LaunchCommand>, agent: Agent) -> Option<AgentCommand> {
    commands.get(agent.id()).map(|c| AgentCommand {
        command: c.command.clone(),
        args: c.args.clone(),
        headless_args: c.headless_args.clone(),
        fallback_command: c.fallback_command.clone(),
        fallback_args: c.fallback_args.clone(),
    })
}

fn machine(host: &LaunchHost) -> Machine {
    let platform = Platform::from_node(&host.platform);
    Machine {
        platform,
        quoting: Quoting::local(platform, &host.shell),
    }
}

/// `buildAgentLaunchLine`.
#[napi(catch_unwind)]
pub fn launch_line(
    payload: LaunchPayload,
    agent_commands: HashMap<String, LaunchCommand>,
    env: HashMap<String, String>,
    host: LaunchHost,
) -> napi::Result<String> {
    let req = request(
        payload,
        "buildAgentLaunchLine called for shell session \u{2014} use createShellPty instead",
    )?;
    let config = command(&agent_commands, req.agent);
    let env: Vec<(String, String)> = env.into_iter().collect();
    launch::launch_line(&req, config.as_ref(), &env, &machine(&host)).map_err(reason)
}

/// `buildHeadlessSpawnArgs`.
#[napi(catch_unwind)]
pub fn headless_args(
    payload: LaunchPayload,
    agent_commands: HashMap<String, LaunchCommand>,
    env: HashMap<String, String>,
    host: LaunchHost,
) -> napi::Result<HeadlessArgs> {
    let req = request(payload, "buildHeadlessSpawnArgs called for shell session")?;
    let config = command(&agent_commands, req.agent);
    let env: Vec<(String, String)> = env.into_iter().collect();
    let spawn =
        launch::headless_spawn(&req, config.as_ref(), &env, &machine(&host)).map_err(reason)?;
    Ok(HeadlessArgs {
        command: spawn.command,
        args: spawn.args,
        stdin: spawn.stdin,
    })
}

/// `tokenize`: the words of a line, or `null` for one it refuses.
#[napi(catch_unwind)]
pub fn launch_tokens(line: String) -> Option<Vec<LaunchToken>> {
    launch::tokenize(&line).map(|tokens| {
        tokens
            .into_iter()
            .map(|t| LaunchToken {
                raw: t.raw.to_owned(),
                value: t.value,
            })
            .collect()
    })
}

/// `getShellIntegration` for `shell`, with its shims written under
/// `shimRoot`; throws when they cannot be written there.
#[napi(catch_unwind)]
pub fn shell_setup(
    shell: String,
    minimal_prompt: bool,
    env: HashMap<String, String>,
    home: String,
    shim_root: String,
) -> napi::Result<ShellSetupResult> {
    let env: Vec<(String, String)> = env.into_iter().collect();
    let cx = shell::ShellContext {
        minimal_prompt,
        env: &env,
        home: &home,
        shim_root: &shim_root,
    };
    let setup = shell::shell_setup(&shell, &cx).map_err(reason)?;
    Ok(ShellSetupResult {
        env: setup.env.into_iter().collect(),
        args: setup.args,
    })
}

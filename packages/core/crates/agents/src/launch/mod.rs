//! What an agent or shell session is started with, worked out without
//! starting anything: the interactive launch line ([`launch_line`]), the
//! headless spawn ([`headless_spawn`]), a shell's integration ([`shell`]),
//! the environment each gets ([`env`]), where a resumed session starts
//! ([`resume_cwd`]) and the name a prompt gives it
//! ([`display_name_from_prompt`]), and a session on a remote host
//! ([`ssh`]).
//!
//! Each answers as the server's TypeScript of the same purpose does
//! (`agent-launch`, `launch-tokens`, `model-arguments`, `resume-cwd`,
//! `shell-integration`, `process-utils`), checked against both by the shared
//! corpus in `tests/fixtures/launch-lines.json`. What the TypeScript reads
//! from its process (the platform, the default shell, the environment) is
//! passed in here, so a host can answer for the machine it runs on and a
//! test for any.
//!
//! Strings are measured in bytes where the TypeScript counts UTF-16 code
//! units. Every place either one cuts a line is ASCII, so the answers agree;
//! where a cut can fall inside a character, the docs say what happens.

use std::borrow::Cow;
use std::fmt;
use std::path::Path;

use crate::{js, paths, Agent, AgentCommand};

pub mod env;
pub mod model;
pub mod shell;
pub mod ssh;
pub mod tokens;

pub use model::{apply_model_arguments, command_shape, CommandShape};
pub use tokens::{strip_session_selectors, tokenize, Token};

/// The kind of machine a launch is worked out for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Posix,
    Windows,
}

impl Platform {
    /// The machine this was built for.
    pub const HOST: Platform = if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Posix
    };

    /// `process.platform` as the server names it.
    pub fn from_node(platform: &str) -> Platform {
        if platform == "win32" {
            Platform::Windows
        } else {
            Platform::Posix
        }
    }
}

/// How a shell reads a quoted word (`shellEscape`'s flavours).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quoting {
    /// Single quotes, `'\''` for a quote inside.
    Posix,
    /// Single quotes, `''` for a quote inside.
    PowerShell,
    /// Double quotes, with `"`, `%` and `^` caret-escaped.
    Cmd,
}

impl Quoting {
    /// The quoting of a terminal running the default shell (`'auto'`): POSIX
    /// off Windows, and on it PowerShell's or cmd's by the default shell's
    /// name.
    pub fn local(platform: Platform, default_shell: &str) -> Quoting {
        match platform {
            Platform::Posix => Quoting::Posix,
            Platform::Windows => {
                let shell = default_shell.to_lowercase();
                if shell.contains("powershell") || shell.contains("pwsh") {
                    Quoting::PowerShell
                } else {
                    Quoting::Cmd
                }
            }
        }
    }

    /// `value` as one word for this shell; unquoted when it is only
    /// characters no shell treats specially (on Windows, `%` is not one).
    pub fn quote(self, value: &str) -> Cow<'_, str> {
        let safe = |c: char| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '.' | '/' | ':' | '=' | '@' | '+' | ',' | '-')
                || (c == '%' && self == Quoting::Posix)
        };
        if !value.is_empty() && value.chars().all(safe) {
            return Cow::Borrowed(value);
        }
        let mut out = String::with_capacity(value.len() + 2);
        match self {
            Quoting::Posix => {
                out.push('\'');
                out.push_str(&value.replace('\'', "'\\''"));
                out.push('\'');
            }
            Quoting::PowerShell => {
                out.push('\'');
                out.push_str(&value.replace('\'', "''"));
                out.push('\'');
            }
            Quoting::Cmd => {
                out.push('"');
                for c in value.chars() {
                    if matches!(c, '"' | '%' | '^') {
                        out.push('^');
                    }
                    out.push(c);
                }
                out.push('"');
            }
        }
        Cow::Owned(out)
    }
}

/// The machine the server runs on, as a launch sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Machine {
    /// Decides how PATH is searched.
    pub platform: Platform,
    /// How the local terminal's shell reads a quoted word
    /// ([`Quoting::local`]). A remote launch is always quoted for POSIX.
    pub quoting: Quoting,
}

/// What a session asks to be launched with: the part of
/// `CreateTerminalPayload` a launch reads. An empty id or prompt counts as
/// none, as JavaScript's truthiness reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    pub agent: Agent,
    /// Arguments for this launch in place of the configured ones.
    pub args: Option<Vec<String>>,
    /// The model to select. Unlike the ids, an empty one is a request, and
    /// refused as an invalid id.
    pub model: Option<String>,
    /// The project's remote host: the line then runs in a POSIX shell on a
    /// machine whose files this one cannot see.
    pub remote_host_id: Option<String>,
    pub resume_session_id: Option<String>,
    /// The id to pin a fresh session to, for agents that take one.
    pub session_id: Option<String>,
    pub initial_prompt: Option<String>,
}

impl LaunchRequest {
    /// A fresh launch of `agent` with nothing else asked.
    pub fn new(agent: Agent) -> LaunchRequest {
        LaunchRequest {
            agent,
            args: None,
            model: None,
            remote_host_id: None,
            resume_session_id: None,
            session_id: None,
            initial_prompt: None,
        }
    }

    fn remote(&self) -> bool {
        given(&self.remote_host_id).is_some()
    }
}

fn given(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|v| !v.is_empty())
}

/// Why a launch cannot be built. Each reads as the server's message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// A model was asked of an agent that has no model choice.
    ModelUnsupported(Agent),
    /// The model id is empty, dash-led, or has spaces or control characters.
    ModelId,
    /// A model flag in the arguments has no value.
    IncompleteModelFlag,
    /// A model was asked with a command that is a wrapper.
    Wrapper,
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::ModelUnsupported(agent) => {
                write!(f, "Model selection is not supported for {}.", agent.id())
            }
            LaunchError::ModelId => f.write_str(
                "Enter a model id without spaces, control characters, or a leading dash.",
            ),
            LaunchError::IncompleteModelFlag => {
                f.write_str("Fix the incomplete model flag in agent arguments.")
            }
            LaunchError::Wrapper => f.write_str(
                "A model needs a command that is one executable. Move the wrapper and its flags into agent arguments, or use the configured default.",
            ),
        }
    }
}

impl std::error::Error for LaunchError {}

/// Where `name` can be run from on `path_env` (`findOnPath`); on Windows
/// `.exe` and `.cmd` count too. Whether a file can be run is this host's
/// answer, as the server's `accessSync` is: on Unix any execute bit.
pub fn find_on_path(name: &str, path_env: Option<&str>, platform: Platform) -> Option<String> {
    let path_env = path_env.filter(|p| !p.is_empty())?;
    let (sep, candidates) = match platform {
        Platform::Windows => (
            ';',
            vec![
                format!("{name}.exe"),
                format!("{name}.cmd"),
                name.to_owned(),
            ],
        ),
        Platform::Posix => (':', vec![name.to_owned()]),
    };
    path_env
        .split(sep)
        .map(js::trim)
        .filter(|dir| !dir.is_empty())
        .flat_map(|dir| candidates.iter().map(move |c| paths::join(dir, c)))
        .find(|full| runnable(Path::new(full)))
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    path.exists()
}

/// The configured command as found (`resolveAgentCommand`): the primary
/// where it is on PATH, else the fallback where that is, else the primary
/// by name.
struct Resolved<'a> {
    command: &'a str,
    args: &'a [String],
    /// Where it was found; the name when it was not.
    path: String,
}

fn resolve<'a>(
    config: &'a AgentCommand,
    env: &[(String, String)],
    platform: Platform,
) -> Resolved<'a> {
    let path_env = env::lookup(env, "PATH").or_else(|| env::lookup(env, "Path"));
    if let Some(path) = find_on_path(&config.command, path_env, platform) {
        return Resolved {
            command: &config.command,
            args: &config.args,
            path,
        };
    }
    if let Some(fallback) = config.fallback_command.as_deref().filter(|f| !f.is_empty()) {
        if let Some(path) = find_on_path(fallback, path_env, platform) {
            return Resolved {
                command: fallback,
                args: config.fallback_args.as_deref().unwrap_or_default(),
                path,
            };
        }
    }
    Resolved {
        command: &config.command,
        args: &config.args,
        path: config.command.clone(),
    }
}

/// The arguments with the chosen model as their one selector, or as given.
fn with_model(
    req: &LaunchRequest,
    command: &str,
    args: &[String],
) -> Result<Vec<String>, LaunchError> {
    match &req.model {
        None => Ok(args.to_vec()),
        Some(model) => {
            model::check_model_command(command, !req.remote())?;
            apply_model_arguments(req.agent, args, model)
        }
    }
}

/// The line typed into a terminal to start the agent interactively
/// (`buildAgentLaunchLine`). `config` is the agent's configured command,
/// the default when `None`; `env` is the launch environment, whose PATH
/// finds the command.
pub fn launch_line(
    req: &LaunchRequest,
    config: Option<&AgentCommand>,
    env: &[(String, String)],
    machine: &Machine,
) -> Result<String, LaunchError> {
    let default;
    let config = match config {
        Some(c) => c,
        None => {
            default = req.agent.default_command();
            &default
        }
    };
    let cmd = resolve(config, env, machine.platform);
    let remote = req.remote();
    let quoting = if remote {
        Quoting::Posix
    } else {
        machine.quoting
    };
    let args = with_model(req, cmd.command, req.args.as_deref().unwrap_or(cmd.args))?;
    // Only a path with spaces that names one file is quoted: `~/bin/claude`
    // keeps its expansion, and a wrapper is read as written.
    let command_line = if cmd.command.chars().any(js::is_space)
        && command_shape(cmd.command, !remote) == CommandShape::Executable
    {
        quoting.quote(cmd.command)
    } else {
        Cow::Borrowed(cmd.command)
    };
    let mut line = String::from(command_line.as_ref());
    for arg in &args {
        line.push(' ');
        line.push_str(&quoting.quote(arg));
    }
    // Where the configured command ends, known exactly because this line was
    // just composed here.
    let args_from = command_line.len();

    let agent = req.agent;
    let resume = given(&req.resume_session_id).filter(|_| agent.resumes_exactly());
    let pin = given(&req.session_id)
        .filter(|_| given(&req.resume_session_id).is_none() && agent.pins_session_ids());

    // A configured command may already carry a selector; two compete and one
    // wins silently. Removed only where it can be proven.
    if resume.is_some() || pin.is_some() {
        line = strip_session_selectors(&line, agent, args_from);
    }

    if let Some(id) = resume {
        let id = quoting.quote(id);
        match agent {
            Agent::Claude | Agent::Copilot => {
                line.push_str(" --resume ");
                line.push_str(&id);
            }
            Agent::Codex => {
                // Spliced in as the first argument, keeping every configured
                // one. The line may have lost blanks at the splice point, so
                // the cut is clamped to it.
                let mut at = args_from.min(line.len());
                while !line.is_char_boundary(at) {
                    at -= 1;
                }
                line.insert_str(at, &format!(" resume {id}"));
            }
            Agent::OpenCode => {
                line.push_str(" --session ");
                line.push_str(&id);
            }
            Agent::Gemini => {}
        }
    }

    if let Some(id) = pin {
        line.push_str(" --session-id ");
        line.push_str(&quoting.quote(id));
    }

    if let Some(prompt) = given(&req.initial_prompt) {
        let prompt = quoting.quote(prompt);
        match agent {
            Agent::Copilot | Agent::Gemini => line.push_str(" -i "),
            Agent::OpenCode => line.push_str(" --prompt "),
            Agent::Claude | Agent::Codex => line.push(' '),
        }
        line.push_str(&prompt);
    }

    Ok(line)
}

/// A headless agent's process (`HeadlessSpawnArgs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadlessSpawn {
    /// Where the command was found, or its name.
    pub command: String,
    pub args: Vec<String>,
    /// The prompt, written to the child's stdin rather than its command line:
    /// on Windows the spawn goes through cmd.exe, which word-splits and
    /// cannot carry a newline.
    pub stdin: Option<String>,
}

/// How to spawn the agent headless, directly and with no shell line
/// (`buildHeadlessSpawnArgs`). Arguments for this launch win over the
/// configured headless ones, which win over the configured ones.
pub fn headless_spawn(
    req: &LaunchRequest,
    config: Option<&AgentCommand>,
    env: &[(String, String)],
    machine: &Machine,
) -> Result<HeadlessSpawn, LaunchError> {
    let default;
    let config = match config {
        Some(c) => c,
        None => {
            default = req.agent.default_command();
            &default
        }
    };
    let cmd = resolve(config, env, machine.platform);
    let base = req
        .args
        .as_deref()
        .or(config.headless_args.as_deref())
        .unwrap_or(cmd.args);
    let mut args = with_model(req, cmd.command, base)?;
    let agent = req.agent;
    let resume = given(&req.resume_session_id);

    match (resume, given(&req.session_id)) {
        (Some(id), _) if matches!(agent, Agent::Claude | Agent::Copilot) => {
            args.extend(["--resume".to_owned(), id.to_owned()]);
        }
        (_, Some(id)) if agent.pins_session_ids() => {
            args.extend(["--session-id".to_owned(), id.to_owned()]);
        }
        _ => {}
    }

    let prompt = given(&req.initial_prompt);
    let spawn = |mut args: Vec<String>, tail: &[&str], stdin: Option<&str>| {
        args.extend(tail.iter().map(|s| (*s).to_owned()));
        HeadlessSpawn {
            command: cmd.path.clone(),
            args,
            stdin: stdin.map(str::to_owned),
        }
    };
    // Each agent reads its prompt from stdin when none is on its command
    // line; with no prompt it is given an empty one so it does not wait there.
    if let (Agent::Codex, Some(id)) = (agent, resume) {
        let tail = ["exec", "resume", id, "-"];
        return Ok(spawn(args, &tail, Some(prompt.unwrap_or(""))));
    }
    Ok(match (agent, prompt) {
        (Agent::Claude, Some(p)) => spawn(args, &["-p"], Some(p)),
        (Agent::Claude, None) => spawn(args, &["-p", ""], None),
        (Agent::Codex, Some(p)) => spawn(args, &["exec"], Some(p)),
        (Agent::Codex, None) => spawn(args, &["exec", ""], None),
        (Agent::OpenCode, Some(p)) => spawn(args, &["run"], Some(p)),
        (Agent::OpenCode, None) => spawn(args, &["run", ""], None),
        (Agent::Copilot | Agent::Gemini, Some(p)) => spawn(args, &[], Some(p)),
        (Agent::Copilot | Agent::Gemini, None) => spawn(args, &["-p", ""], None),
    })
}

/// The name a prompt gives a session (`displayNameFromPrompt`): whitespace
/// collapsed, cut to `max_len` UTF-16 code units at the last space with an
/// ellipsis; `None` when there is nothing in it. A cut with no space before
/// it that would split a surrogate pair leaves the pair out, where the
/// TypeScript keeps a lone surrogate a Rust string cannot hold.
pub fn display_name_from_prompt(prompt: &str, max_len: usize) -> Option<String> {
    let cleaned = prompt
        .split(js::is_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.encode_utf16().count() <= max_len {
        return Some(cleaned);
    }
    let truncated = js::slice_utf16(&cleaned, max_len);
    let mut name = match truncated.rfind(' ') {
        Some(at) if at > 0 => cleaned[..at].to_owned(),
        _ => truncated,
    };
    name.push('\u{2026}');
    Some(name)
}

/// Where a session resumes (`ResumeCwd`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeCwd {
    pub cwd: String,
    /// The more specific place the session wanted, when it is gone.
    pub fell_back_from: Option<String>,
}

/// The most specific of the shell's own directory, the worktree and the
/// project that is still a directory (`resumeCwdFor`); `None` when none is,
/// so nothing is spawned into a directory that is not there.
pub fn resume_cwd(
    shell_cwd: Option<&str>,
    worktree_path: Option<&str>,
    project_path: Option<&str>,
    is_directory: impl Fn(&str) -> bool,
) -> Option<ResumeCwd> {
    let wanted: Vec<&str> = [shell_cwd, worktree_path, project_path]
        .into_iter()
        .flatten()
        .collect();
    let at = wanted.iter().position(|w| is_directory(w))?;
    Some(ResumeCwd {
        cwd: wanted[at].to_owned(),
        fell_back_from: (at > 0).then(|| wanted[0].to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSIX: Machine = Machine {
        platform: Platform::Posix,
        quoting: Quoting::Posix,
    };

    #[test]
    fn quotes_for_each_shell() {
        assert_eq!(Quoting::Posix.quote("a-b/c:d%"), "a-b/c:d%");
        assert_eq!(Quoting::Posix.quote(""), "''");
        assert_eq!(Quoting::Posix.quote("it's"), "'it'\\''s'");
        assert_eq!(Quoting::Cmd.quote("50%"), "\"50^%\"");
        assert_eq!(Quoting::Cmd.quote("a\"b^"), "\"a^\"b^^\"");
        assert_eq!(Quoting::PowerShell.quote("it's"), "'it''s'");
        assert_eq!(Quoting::Posix.quote("é"), "'é'");
    }

    #[test]
    fn reads_the_default_shell_for_quoting_on_windows_only() {
        assert_eq!(Quoting::local(Platform::Posix, "pwsh"), Quoting::Posix);
        assert_eq!(
            Quoting::local(
                Platform::Windows,
                "C:\\Program Files\\PowerShell\\7\\PWSH.exe"
            ),
            Quoting::PowerShell
        );
        assert_eq!(Quoting::local(Platform::Windows, "cmd.exe"), Quoting::Cmd);
    }

    #[test]
    fn splices_codex_resume_where_blanks_were_taken() {
        let mut req = LaunchRequest::new(Agent::Codex);
        req.resume_session_id = Some("id".into());
        // The configured resume and its blanks go, taking the line to before
        // where the command ended.
        let config = AgentCommand {
            command: "npx codex  ".into(),
            args: vec!["resume".into(), "old".into()],
            ..Agent::Codex.default_command()
        };
        let line = launch_line(&req, Some(&config), &[], &POSIX).unwrap();
        assert_eq!(line, "npx codex resume id");
    }

    #[test]
    fn names_a_session_from_its_prompt() {
        assert_eq!(display_name_from_prompt(" \n ", 60), None);
        assert_eq!(
            display_name_from_prompt("\n\u{FEFF} Fix   the\n\nbug ", 60).as_deref(),
            Some("Fix the bug")
        );
        assert_eq!(
            display_name_from_prompt("aaaa bbbb cccc", 7).as_deref(),
            Some("aaaa\u{2026}")
        );
        assert_eq!(
            display_name_from_prompt("abcdefgh", 4).as_deref(),
            Some("abcd\u{2026}")
        );
        // The cut falls inside the emoji's surrogate pair.
        assert_eq!(
            display_name_from_prompt("abc😀def", 4).as_deref(),
            Some("abc\u{2026}")
        );
    }

    #[test]
    fn resumes_into_the_most_specific_place_still_there() {
        let has = |dirs: &'static [&'static str]| move |at: &str| dirs.contains(&at);
        assert_eq!(
            resume_cwd(
                Some("/r/w/s"),
                Some("/r/w"),
                Some("/r"),
                has(&["/r", "/r/w"])
            ),
            Some(ResumeCwd {
                cwd: "/r/w".into(),
                fell_back_from: Some("/r/w/s".into())
            })
        );
        assert_eq!(
            resume_cwd(None, None, Some("/r"), has(&["/r"])),
            Some(ResumeCwd {
                cwd: "/r".into(),
                fell_back_from: None
            })
        );
        assert_eq!(resume_cwd(None, Some("/r/w"), Some("/r"), has(&[])), None);
    }
}

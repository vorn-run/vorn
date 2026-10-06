//! A chosen model on an agent's arguments (`model-arguments`): the
//! configured arguments with the model as their one selector, and whether a
//! command is one executable a model can ride.

use std::path::Path;

use super::tokens::tokenize;
use super::LaunchError;
use crate::{js, Agent};

/// What a configured command is to a shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandShape {
    /// One file the shell can be handed quoted.
    Executable,
    /// Words the shell must read as written: a wrapper and its flags.
    Wrapper,
}

/// `commandShape`. A path with spaces splits into several words yet names
/// one file, which only this machine can tell, so a remote command of
/// several words is always a wrapper.
pub fn command_shape(command: &str, on_this_machine: bool) -> CommandShape {
    let operator = command
        .bytes()
        .any(|b| matches!(b, b';' | b'&' | b'|' | b'<' | b'>' | b'`' | b'\n' | b'\r'))
        || command.contains("$(");
    if operator {
        return CommandShape::Wrapper;
    }
    match tokenize(command).map(|t| t.len()) {
        None => CommandShape::Wrapper,
        Some(1) => CommandShape::Executable,
        Some(_) if on_this_machine && Path::new(command).exists() => CommandShape::Executable,
        Some(_) => CommandShape::Wrapper,
    }
}

/// `assertModelCommand`: a model rides the arguments, so the command must be
/// one executable.
pub fn check_model_command(command: &str, on_this_machine: bool) -> Result<(), LaunchError> {
    match command_shape(command, on_this_machine) {
        CommandShape::Executable => Ok(()),
        CommandShape::Wrapper => Err(LaunchError::Wrapper),
    }
}

/// `validateModelId`: the id trimmed, refused when empty, dash-led, or with
/// whitespace or control characters in it.
pub fn validate_model_id(value: &str) -> Result<&str, LaunchError> {
    let model = js::trim(value);
    let unprintable = model.chars().any(|c| (c as u32) < 32 || c as u32 == 127);
    if model.is_empty() || model.starts_with('-') || model.chars().any(js::is_space) || unprintable
    {
        return Err(LaunchError::ModelId);
    }
    Ok(model)
}

/// `^model\s*=`, codex's config override for the model.
fn is_codex_model_config(value: &str) -> bool {
    value
        .strip_prefix("model")
        .is_some_and(|rest| rest.trim_start_matches(js::is_space).starts_with('='))
}

/// How many arguments the model selector at `args[i]` spans; 0 when it is
/// not one.
fn selector_len(agent: Agent, args: &[String], i: usize) -> Result<usize, LaunchError> {
    let arg = args[i].as_str();
    let next = args.get(i + 1).map(String::as_str);
    // Every agent takes `--model`; all but claude also `-m`.
    let short = agent != Agent::Claude;
    if arg == "--model" || (short && arg == "-m") {
        return match next {
            Some(n) if !n.is_empty() && !n.starts_with('-') => Ok(2),
            _ => Err(LaunchError::IncompleteModelFlag),
        };
    }
    if arg.starts_with("--model=") || (short && arg.starts_with("-m") && arg.len() > 2) {
        return Ok(1);
    }
    if agent == Agent::Codex {
        if (arg == "-c" || arg == "--config") && next.is_some_and(is_codex_model_config) {
            return Ok(2);
        }
        // `^(?:-c=?|--config=)model\s*=`
        let joined = ["-c=", "-c", "--config="]
            .iter()
            .filter_map(|p| arg.strip_prefix(p))
            .any(is_codex_model_config);
        if joined {
            return Ok(1);
        }
    }
    Ok(0)
}

/// `applyModelArguments`: the configured arguments with `model` as their
/// one model selector, appended; nothing past `--` is touched.
pub fn apply_model_arguments(
    agent: Agent,
    args: &[String],
    model: &str,
) -> Result<Vec<String>, LaunchError> {
    if !agent.selects_models() {
        return Err(LaunchError::ModelUnsupported(agent));
    }
    let id = validate_model_id(model)?;
    let mut kept = Vec::with_capacity(args.len() + 2);
    let mut i = 0;
    while i < args.len() && args[i] != "--" {
        match selector_len(agent, args, i)? {
            0 => {
                kept.push(args[i].clone());
                i += 1;
            }
            n => i += n,
        }
    }
    kept.push("--model".to_owned());
    kept.push(id.to_owned());
    kept.extend_from_slice(&args[i..]);
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn removes_codex_model_overrides_but_keeps_sandbox_settings() {
        let args = strings(&[
            "-mold",
            "-c",
            "model=\"old\"",
            "--config=model=\"other\"",
            "-cmodel =x",
            "-a",
            "never",
            "-s",
            "read-only",
        ]);
        assert_eq!(
            apply_model_arguments(Agent::Codex, &args, "new").unwrap(),
            ["-a", "never", "-s", "read-only", "--model", "new"]
        );
    }

    #[test]
    fn leaves_claude_short_flags_alone() {
        let args = strings(&["-m", "x", "--model=y"]);
        assert_eq!(
            apply_model_arguments(Agent::Claude, &args, "new").unwrap(),
            ["-m", "x", "--model", "new"]
        );
    }

    #[test]
    fn refuses_incomplete_flags_and_unsafe_ids() {
        let args = strings(&["--model", "--sandbox"]);
        assert_eq!(
            apply_model_arguments(Agent::Codex, &args, "new"),
            Err(LaunchError::IncompleteModelFlag)
        );
        assert_eq!(
            apply_model_arguments(Agent::Codex, &strings(&["-m", ""]), "new"),
            Err(LaunchError::IncompleteModelFlag)
        );
        for id in ["", "-flag", "bad\nmodel", "a b", "a\u{7f}"] {
            assert_eq!(validate_model_id(id), Err(LaunchError::ModelId), "{id:?}");
        }
        assert_eq!(validate_model_id(" opus[1m] \u{FEFF}"), Ok("opus[1m]"));
        assert_eq!(
            apply_model_arguments(Agent::Gemini, &[], "x"),
            Err(LaunchError::ModelUnsupported(Agent::Gemini))
        );
    }

    #[test]
    fn knows_a_wrapper_from_one_executable() {
        assert_eq!(command_shape("claude", true), CommandShape::Executable);
        assert_eq!(
            command_shape("'/opt/my tools/claude'", false),
            CommandShape::Executable
        );
        for wrapper in ["npx -y codex", "echo x && codex", "a\rb", "a(b", "", "  "] {
            assert_eq!(
                command_shape(wrapper, true),
                CommandShape::Wrapper,
                "{wrapper:?}"
            );
        }
    }

    #[test]
    fn a_spaced_path_is_one_executable_only_on_this_machine() {
        let dir = tempfile::tempdir().unwrap();
        let spaced = dir.path().join("my tools");
        std::fs::write(&spaced, "").unwrap();
        let spaced = spaced.to_str().unwrap();
        assert_eq!(command_shape(spaced, true), CommandShape::Executable);
        assert_eq!(command_shape(spaced, false), CommandShape::Wrapper);
    }
}

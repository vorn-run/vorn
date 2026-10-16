//! A chosen model on an agent's arguments: the configured arguments with the
//! model as their one selector, and whether a command is one executable a
//! model can ride.

use std::path::Path;

use super::tokens::tokenize;
use super::LaunchError;
use crate::spec::Models;
use crate::{js, Agent};

/// What a configured command is to a shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandShape {
    /// One file the shell can be handed quoted.
    Executable,
    /// Words the shell must read as written: a wrapper and its flags.
    Wrapper,
}

/// Whether a command is one executable or a wrapper. A path with spaces
/// splits into several words yet names one file, which only this machine can
/// tell, so a remote command of several words is always a wrapper.
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

/// A model rides the arguments, so the command must be
/// one executable.
pub fn check_model_command(command: &str, on_this_machine: bool) -> Result<(), LaunchError> {
    match command_shape(command, on_this_machine) {
        CommandShape::Executable => Ok(()),
        CommandShape::Wrapper => Err(LaunchError::Wrapper),
    }
}

/// The model id trimmed, refused when empty, dash-led, or with
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

/// `^<key>\s*=`: a config override setting `key`.
fn sets_key(value: &str, key: &str) -> bool {
    value
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start_matches(js::is_space).starts_with('='))
}

/// Whether `flag` is one dash and one letter, which also takes its value
/// joined on (`-mvalue`).
fn short(flag: &str) -> bool {
    flag.len() == 2 && flag.starts_with('-') && flag != "--"
}

/// How many arguments the model selector at `args[i]` spans; 0 when it is
/// not one.
fn selector_len(models: &Models, args: &[String], i: usize) -> Result<usize, LaunchError> {
    let arg = args[i].as_str();
    let next = args.get(i + 1).map(String::as_str);
    let flags = std::iter::once(&models.flag).chain(&models.selectors);
    for flag in flags.clone() {
        if arg == flag {
            return match next {
                Some(n) if !n.is_empty() && !n.starts_with('-') => Ok(2),
                _ => Err(LaunchError::IncompleteModelFlag),
            };
        }
    }
    for flag in flags {
        let joined = if short(flag) {
            arg.starts_with(flag.as_str()) && arg.len() > 2
        } else {
            arg.strip_prefix(flag.as_str())
                .is_some_and(|rest| rest.starts_with('='))
        };
        if joined {
            return Ok(1);
        }
    }
    if let Some(config) = &models.config {
        let key = config.key.as_str();
        if config.flags.iter().any(|f| f == arg) && next.is_some_and(|n| sets_key(n, key)) {
            return Ok(2);
        }
        // `-c=key=`, `-ckey=`, `--config=key=`.
        let joined = config.flags.iter().any(|f| {
            let prefixes: &[String] = &if short(f) {
                vec![format!("{f}="), f.clone()]
            } else {
                vec![format!("{f}=")]
            };
            prefixes
                .iter()
                .filter_map(|p| arg.strip_prefix(p.as_str()))
                .any(|rest| sets_key(rest, key))
        });
        if joined {
            return Ok(1);
        }
    }
    Ok(0)
}

/// The configured arguments with `model` as their one model selector,
/// appended; nothing past `--` is touched.
pub fn apply_model_arguments(
    agent: &Agent,
    args: &[String],
    model: &str,
) -> Result<Vec<String>, LaunchError> {
    let Some(models) = &agent.spec().models else {
        return Err(LaunchError::ModelUnsupported(agent.id().to_owned()));
    };
    let id = validate_model_id(model)?;
    let mut kept = Vec::with_capacity(args.len() + 2);
    let mut i = 0;
    while i < args.len() && args[i] != "--" {
        match selector_len(models, args, i)? {
            0 => {
                kept.push(args[i].clone());
                i += 1;
            }
            n => i += n,
        }
    }
    kept.push(models.flag.clone());
    kept.push(id.to_owned());
    kept.extend_from_slice(&args[i..]);
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{agent, bare};

    /// A model flag with a short form and a config override, and one without either.
    fn configured() -> Agent {
        agent(
            "[agent]\nid = \"cfg\"\nname = \"C\"\n[agent.launch]\ncommand = \"c\"\n\
             [agent.models]\nselectors = [\"-m\"]\nconfig = { flags = [\"-c\", \"--config\"], key = \"model\" }\n",
        )
    }

    fn long_only() -> Agent {
        agent("[agent]\nid = \"long\"\nname = \"L\"\n[agent.launch]\ncommand = \"l\"\n[agent.models]\n")
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn removes_config_model_overrides_but_keeps_other_settings() {
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
            apply_model_arguments(&configured(), &args, "new").unwrap(),
            ["-a", "never", "-s", "read-only", "--model", "new"]
        );
    }

    #[test]
    fn leaves_short_flags_alone_where_the_agent_has_none() {
        let args = strings(&["-m", "x", "--model=y"]);
        assert_eq!(
            apply_model_arguments(&long_only(), &args, "new").unwrap(),
            ["-m", "x", "--model", "new"]
        );
    }

    #[test]
    fn refuses_incomplete_flags_and_unsafe_ids() {
        let args = strings(&["--model", "--sandbox"]);
        assert_eq!(
            apply_model_arguments(&configured(), &args, "new"),
            Err(LaunchError::IncompleteModelFlag)
        );
        assert_eq!(
            apply_model_arguments(&configured(), &strings(&["-m", ""]), "new"),
            Err(LaunchError::IncompleteModelFlag)
        );
        for id in ["", "-flag", "bad\nmodel", "a b", "a\u{7f}"] {
            assert_eq!(validate_model_id(id), Err(LaunchError::ModelId), "{id:?}");
        }
        assert_eq!(validate_model_id(" opus[1m] \u{FEFF}"), Ok("opus[1m]"));
        assert_eq!(
            apply_model_arguments(&bare("plain"), &[], "x"),
            Err(LaunchError::ModelUnsupported("plain".into()))
        );
    }

    #[test]
    fn knows_a_wrapper_from_one_executable() {
        assert_eq!(command_shape("agent", true), CommandShape::Executable);
        assert_eq!(
            command_shape("'/opt/my tools/agent'", false),
            CommandShape::Executable
        );
        for wrapper in ["npx -y agent", "echo x && agent", "a\rb", "a(b", "", "  "] {
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

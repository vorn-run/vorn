//! `vorn_agents::launch` against the corpus the server's TypeScript is
//! checked against too: `tests/fixtures/launch-lines.json` at the repository
//! root. `tests/launch-parity.test.ts` checks the TypeScript against it and
//! puts both through the same cases and random lines besides.
//!
//! Cases that need executables in `{bin}` run on Unix, where their expected
//! paths are spelled; shells whose integration reads the server's shim files
//! are left to the parity test, which has the server write them.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{json, Value};
use vorn_agents::launch::{self, env, shell, LaunchRequest, Machine, Platform, Quoting};
use vorn_agents::{Agent, AgentCommand};

fn corpus() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/launch-lines.json");
    let text = std::fs::read_to_string(&path).expect("the corpus is checked in");
    serde_json::from_str(&text).expect("the corpus is JSON")
}

fn string(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn strings(v: &Value) -> Option<Vec<String>> {
    v.as_array()
        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
}

/// Every string in `value` with each `{name}` filled in.
fn fill(value: &Value, vars: &HashMap<&str, String>) -> Value {
    match value {
        Value::String(s) => {
            let mut s = s.clone();
            for (name, at) in vars {
                s = s.replace(&format!("{{{name}}}"), at);
            }
            Value::String(s)
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| fill(v, vars)).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), fill(v, vars))).collect())
        }
        other => other.clone(),
    }
}

fn command(v: &Value) -> AgentCommand {
    AgentCommand {
        command: string(&v["command"]).unwrap(),
        args: strings(&v["args"]).unwrap(),
        headless_args: strings(&v["headlessArgs"]),
        fallback_command: string(&v["fallbackCommand"]),
        fallback_args: strings(&v["fallbackArgs"]),
    }
}

fn outcome<T>(result: Result<T, launch::LaunchError>, ok: impl Fn(T) -> Value) -> Value {
    match result {
        Ok(v) => ok(v),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

#[test]
fn launches_as_the_corpus_says() {
    let corpus = corpus();
    let mut checked = 0;
    for case in corpus["launch"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bin_names = strings(&case["bin"]).unwrap_or_default();
        if !bin_names.is_empty() && !cfg!(unix) {
            continue;
        }
        let Some(agent) = Agent::from_id(case["payload"]["agentType"].as_str().unwrap()) else {
            // A shell session is refused by the adapter, before the crate.
            continue;
        };
        let bin = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        for file in &bin_names {
            use std::os::unix::fs::PermissionsExt;
            let path = bin.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let vars = HashMap::from([("bin", bin.path().to_str().unwrap().to_owned())]);
        let case = fill(case, &vars);

        let p = &case["payload"];
        let req = LaunchRequest {
            agent,
            args: strings(&p["args"]),
            model: string(&p["model"]),
            remote_host_id: string(&p["remoteHostId"]),
            resume_session_id: string(&p["resumeSessionId"]),
            session_id: string(&p["sessionId"]),
            initial_prompt: string(&p["initialPrompt"]),
        };
        let config = case["commands"].get(agent.id()).map(command);
        let env: Vec<(String, String)> = match case["env"].as_object() {
            Some(env) => env
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                .collect(),
            None => {
                let path = string(&case["path"]).unwrap_or_else(|| {
                    if bin_names.is_empty() {
                        "/nonexistent-vorn-bin".to_owned()
                    } else {
                        vars["bin"].clone()
                    }
                });
                vec![("PATH".to_owned(), path)]
            }
        };
        let platform = Platform::from_node(case["platform"].as_str().unwrap());
        let shell = case["shell"].as_str().unwrap_or("/bin/zsh");
        let machine = Machine {
            platform,
            quoting: Quoting::local(platform, shell),
        };

        let line = outcome(
            launch::launch_line(&req, config.as_ref(), &env, &machine),
            Value::String,
        );
        assert_eq!(line, case["line"], "line: {name}");
        let headless = outcome(
            launch::headless_spawn(&req, config.as_ref(), &env, &machine),
            |h| match h.stdin {
                Some(stdin) => json!({ "command": h.command, "args": h.args, "stdin": stdin }),
                None => json!({ "command": h.command, "args": h.args }),
            },
        );
        assert_eq!(headless, case["headless"], "headless: {name}");
        checked += 1;
    }
    assert!(checked > 100, "only {checked} launch cases ran");
}

#[test]
fn reads_and_refuses_lines_as_the_corpus_says() {
    let corpus = corpus();
    for case in corpus["tokens"].as_array().unwrap() {
        let line = case["line"].as_str().unwrap();
        let got = launch::tokenize(line).map(|tokens| {
            tokens
                .into_iter()
                .map(|t| json!({ "raw": t.raw, "value": t.value }))
                .collect::<Vec<_>>()
        });
        assert_eq!(json!(got), case["tokens"], "{line:?}");
    }
}

#[test]
fn strips_selectors_as_the_corpus_says() {
    let corpus = corpus();
    for case in corpus["strip"].as_array().unwrap() {
        let line = case["line"].as_str().unwrap();
        let agent = Agent::from_id(case["agent"].as_str().unwrap()).unwrap();
        let from = case["command"].as_str().unwrap().len();
        assert_eq!(
            launch::strip_session_selectors(line, agent, from),
            case["expected"].as_str().unwrap(),
            "{line:?}"
        );
    }
}

#[test]
fn names_resumes_and_filters_as_the_corpus_says() {
    let corpus = corpus();
    for case in corpus["displayNames"].as_array().unwrap() {
        let max = case["maxLen"].as_u64().map_or(60, |n| n as usize);
        let got = launch::display_name_from_prompt(case["prompt"].as_str().unwrap(), max);
        assert_eq!(json!(got), case["name"], "{:?}", case["prompt"]);
    }

    for case in corpus["resumeCwd"].as_array().unwrap() {
        let s = &case["session"];
        let dirs = strings(&case["dirs"]).unwrap();
        let got = launch::resume_cwd(
            s["shellCwd"].as_str(),
            s["worktreePath"].as_str(),
            s["projectPath"].as_str(),
            |at| dirs.iter().any(|d| d == at),
        );
        let got = got.map(|r| match r.fell_back_from {
            Some(from) => json!({ "cwd": r.cwd, "fellBackFrom": from }),
            None => json!({ "cwd": r.cwd }),
        });
        assert_eq!(json!(got), case["result"], "{s}");
    }

    let pairs = |v: &Value| -> Vec<(String, String)> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p[0].as_str().unwrap().to_owned(),
                    p[1].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    for case in corpus["env"].as_array().unwrap() {
        let source = pairs(&case["source"]);
        let passthrough = strings(&case["passthrough"]).unwrap();
        assert_eq!(env::safe_env(source.clone()), pairs(&case["safe"]));
        assert_eq!(
            env::launch_env(source, &passthrough, case["dataDir"].as_str()),
            pairs(&case["launch"])
        );
    }
}

#[test]
fn sets_up_shells_without_shims_as_the_corpus_says() {
    let corpus = corpus();
    let mut checked = 0;
    for case in corpus["shell"].as_array().unwrap() {
        let path = case["shell"].as_str().unwrap();
        if matches!(
            shell::ShellFamily::of(path),
            Some(shell::ShellFamily::Zsh | shell::ShellFamily::Bash | shell::ShellFamily::Fish)
        ) {
            continue;
        }
        let env: Vec<(String, String)> = case["env"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        let cx = shell::ShellContext {
            minimal_prompt: case["minimalPrompt"].as_bool().unwrap(),
            env: &env,
            home: case["home"].as_str().unwrap(),
            shim_root: "/nonexistent-vorn-shims",
        };
        let setup = shell::shell_setup(path, &cx).unwrap();
        let env: serde_json::Map<String, Value> = setup
            .env
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        assert_eq!(
            json!({ "env": env, "args": setup.args }),
            case["setup"],
            "{path}"
        );
        checked += 1;
    }
    assert!(checked >= 10, "only {checked} shell cases ran");
}

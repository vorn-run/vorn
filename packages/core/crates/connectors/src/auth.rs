//! The sign-in rung a connector declared, acted on: whether the tool it
//! borrows is signed in, and the token that tool hands over at spawn.
//!
//! What a manifest may borrow is held to three rules it cannot talk past: it
//! declared reading the variable, the variable is not stripped for every
//! child, and it is not a credential by name. Streams a tool prints can hold
//! a token, so only what went wrong is ever logged, never what was printed.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};
use tracing::warn;
use vorn_agents::launch::env::{is_stripped, SENSITIVE_PREFIXES};

/// Long enough for a cold CLI, short enough that a form does not hang on it.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Names no third-party process is handed, whatever it declares.
const NEVER_BORROWED_KEYS: [&str; 1] = ["CLAUDECODE"];
const NEVER_BORROWED_PREFIXES: [&str; 1] = ["CLAUDE_CODE_"];

/// What runs a connector's sign-in tool, so a test can answer for one.
pub trait Runner: Send + Sync {
    /// The executable `name` resolves to, if it is installed.
    fn resolve(&self, name: &str) -> Option<PathBuf>;
    /// Runs it, answering stdout and stderr, or why it failed (first line only).
    fn run(
        &self,
        file: &PathBuf,
        args: &[String],
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<(String, String), String>;
    /// This machine's value of `name`.
    fn var(&self, name: &str) -> Option<String>;
    /// The environment every child starts from.
    fn safe_env(&self) -> Vec<(String, String)>;
}

/// The auth a connection may act on: its manifest's auth block, the
/// variables it declares reading, and whether the app wrote the block.
#[derive(Debug, Clone, Default)]
pub struct Source {
    pub auth: Option<Value>,
    pub declared: Vec<String>,
    pub trusted: bool,
}

fn borrow_env(auth: Option<&Value>) -> Vec<String> {
    auth.and_then(|a| a.pointer("/borrow/env"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn never_borrowed(name: &str) -> bool {
    let upper = name.to_uppercase();
    NEVER_BORROWED_KEYS.contains(&upper.as_str())
        || NEVER_BORROWED_PREFIXES.iter().any(|p| upper.starts_with(p))
}

fn credential_name(name: &str) -> bool {
    let upper = name.to_uppercase();
    SENSITIVE_PREFIXES.iter().any(|p| upper.starts_with(p))
}

/// The borrow names a manifest can honour, in the casing it declared them.
pub fn declared_borrows(source: &Source) -> Vec<String> {
    borrow_env(source.auth.as_ref())
        .iter()
        .filter_map(|asked| {
            source
                .declared
                .iter()
                .rev()
                .find(|d| d.to_uppercase() == asked.to_uppercase())
                .cloned()
        })
        .collect()
}

/// What a connection may actually borrow (`borrowableNames`).
pub fn borrowable_names(source: &Source) -> Vec<String> {
    let honoured = declared_borrows(source);
    let allowed: Vec<String> = honoured
        .into_iter()
        .filter(|n| source.trusted || (!never_borrowed(n) && !credential_name(n)))
        .filter(|n| !is_stripped(n))
        .collect();
    for asked in borrow_env(source.auth.as_ref()) {
        if !allowed
            .iter()
            .any(|n| n.to_uppercase() == asked.to_uppercase())
        {
            warn!("[auth] refused to borrow {asked}: undeclared, stripped for everyone, or a credential by name");
        }
    }
    allowed
}

fn borrowed_env(runner: &dyn Runner, names: &[String]) -> Vec<(String, String)> {
    let mut env = runner.safe_env();
    for name in names {
        env.retain(|(k, _)| k != name);
        if let Some(value) = runner.var(name).filter(|v| !v.is_empty()) {
            env.push((name.clone(), value));
        }
    }
    env
}

/// Text without its terminal escapes.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || (c == '\u{1b}' && chars.peek() == Some(&'\\')) {
                        if c == '\u{1b}' {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            _ => {
                chars.next();
            }
        }
    }
    out
}

/// Who a status command says is signed in, only from a phrase that names
/// somebody and is not a refusal (`identityFrom`).
pub fn identity_from(output: &str) -> Option<String> {
    let text = strip_ansi(output);
    let negation = [
        "not", "cannot", "can't", "couldn't", "failed", "unable", "denied", "expired", "invalid",
    ];
    for phrase in ["account", "as"] {
        let lower = text.to_lowercase();
        let mut from = 0;
        let found = loop {
            let Some(at) = lower[from..].find(phrase).map(|i| i + from) else {
                break None;
            };
            let before_ok = at == 0
                || !lower[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
            let rest = &text[at + phrase.len()..];
            let spaced = rest.chars().next().is_some_and(char::is_whitespace);
            if before_ok && spaced {
                let word: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| !c.is_whitespace())
                    .collect();
                if !word.is_empty() {
                    break Some((at, word));
                }
            }
            from = at + phrase.len();
        };
        let Some((at, word)) = found else { continue };
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let lead = text[line_start..at].to_lowercase();
        let refused = lead
            .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '_'))
            .any(|w| negation.contains(&w));
        if refused {
            continue;
        }
        return Some(word.trim_end_matches(['.', ',', ')']).to_owned());
    }
    None
}

/// What to run to sign in: a status command's `login`, else the tool itself.
pub fn sign_in_command(auth: &Value) -> String {
    let command = auth
        .pointer("/probe/command")
        .and_then(Value::as_str)
        .unwrap_or("");
    let args: Vec<&str> = auth
        .pointer("/probe/args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    match args.split_last() {
        Some((&"status", rest)) => std::iter::once(command)
            .chain(rest.iter().copied())
            .chain(std::iter::once("login"))
            .collect::<Vec<_>>()
            .join(" "),
        _ => command.to_owned(),
    }
}

/// How to install a tool this build can say something useful about.
pub fn install_hint(command: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let win = cfg!(windows);
    match command {
        "gh" if mac => "Install with Homebrew: `brew install gh`".into(),
        "gh" if win => "Install with winget: `winget install --id GitHub.cli` (or download from https://cli.github.com)".into(),
        "gh" => "Install from https://cli.github.com (Debian/Ubuntu: `sudo apt install gh`)".into(),
        "glab" if mac => "Install with Homebrew: `brew install glab`".into(),
        "glab" if win => "Install with winget: `winget install --id GitLab.glab`".into(),
        "glab" => "Install from https://gitlab.com/gitlab-org/cli".into(),
        other => format!("Install `{other}` and make sure it is on your PATH."),
    }
}

fn probe_parts(auth: &Value) -> (Option<String>, Vec<String>) {
    let command = auth
        .pointer("/probe/command")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(str::to_owned);
    let args = auth
        .pointer("/probe/args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    (command, args)
}

/// Whether a connector's sign-in tool is signed in (`probeAuth`):
/// `{ok, identity?, message?, installHint?}`, `ok` null for every rung but
/// `cli`, which has nothing to ask.
pub fn probe(source: &Source, runner: &dyn Runner) -> Value {
    let Some(auth) = source.auth.as_ref() else {
        return json!({ "ok": null });
    };
    let rung = auth.get("rung").and_then(Value::as_str);
    if rung == Some("none") {
        return json!({ "ok": true, "message": "Nothing to sign in to — installing it is the whole setup." });
    }
    let (Some("cli"), (Some(command), args)) = (rung, probe_parts(auth)) else {
        return json!({ "ok": null });
    };
    let Some(resolved) = runner.resolve(&command) else {
        return json!({
            "ok": false,
            "message": format!("{command} is not installed or not on PATH."),
            "installHint": install_hint(&command),
        });
    };
    let env = borrowed_env(runner, &borrowable_names(source));
    match runner.run(&resolved, &args, &env, PROBE_TIMEOUT) {
        Ok((stdout, stderr)) => match identity_from(&format!("{stdout}\n{stderr}")) {
            Some(identity) => json!({ "ok": true, "identity": identity }),
            None => json!({ "ok": true }),
        },
        Err(why) => {
            warn!("[auth] {command} reported no session: {why}");
            json!({ "ok": false, "message": format!("Sign in by running `{}` in your terminal.", sign_in_command(auth)) })
        }
    }
}

/// What a `cli` connector's child starts with: the names it may borrow that
/// this machine has, and its token fetched fresh (`borrowedSecrets`).
pub fn borrowed_secrets(source: &Source, runner: &dyn Runner) -> Vec<(String, String)> {
    let Some(auth) = source
        .auth
        .as_ref()
        .filter(|a| a.get("rung").and_then(Value::as_str) == Some("cli"))
    else {
        return Vec::new();
    };
    let names = borrowable_names(source);
    if names.is_empty() {
        return Vec::new();
    }
    let mut borrowed: Vec<(String, String)> = names
        .iter()
        .filter_map(|n| {
            runner
                .var(n)
                .filter(|v| !v.is_empty())
                .map(|v| (n.clone(), v))
        })
        .collect();
    let asked = auth
        .pointer("/borrow/tokenEnv")
        .and_then(Value::as_str)
        .unwrap_or(&names[0])
        .to_uppercase();
    let target = names.iter().find(|n| n.to_uppercase() == asked).cloned();
    let token_args: Vec<String> = auth
        .pointer("/borrow/tokenArgs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let (command, _) = probe_parts(auth);
    let Some(target) = target.filter(|t| !borrowed.iter().any(|(k, _)| k == t)) else {
        return borrowed;
    };
    let (Some(command), false) = (command, token_args.is_empty()) else {
        return borrowed;
    };
    let Some(resolved) = runner.resolve(&command) else {
        return borrowed;
    };
    let env = borrowed_env(runner, &names);
    match runner.run(&resolved, &token_args, &env, PROBE_TIMEOUT) {
        Ok((stdout, _)) => {
            let token = stdout.trim();
            if !token.is_empty() {
                borrowed.push((target, token.to_owned()));
            }
        }
        Err(why) => warn!("[auth] {command} could not hand over a token: {why}"),
    }
    borrowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        installed: bool,
        answer: Option<Result<(String, String), String>>,
        ran: Mutex<Vec<Vec<String>>>,
    }

    impl Runner for Fake {
        fn resolve(&self, name: &str) -> Option<PathBuf> {
            self.installed
                .then(|| PathBuf::from(format!("/bin/{name}")))
        }
        fn run(
            &self,
            _: &PathBuf,
            args: &[String],
            _: &[(String, String)],
            _: Duration,
        ) -> Result<(String, String), String> {
            self.ran.lock().unwrap().push(args.to_vec());
            self.answer.clone().unwrap_or(Err("exit 1".into()))
        }
        fn var(&self, name: &str) -> Option<String> {
            (name == "GH_HOST").then(|| "github.com".into())
        }
        fn safe_env(&self) -> Vec<(String, String)> {
            vec![("PATH".into(), "/bin".into())]
        }
    }

    fn cli() -> Source {
        Source {
            auth: Some(
                json!({ "rung": "cli", "probe": { "command": "gh", "args": ["auth", "status"] },
                               "borrow": { "env": ["GH_HOST", "MY_TOKEN", "GITHUB_TOKEN"], "tokenEnv": "my_token", "tokenArgs": ["auth", "token"] } }),
            ),
            declared: vec!["GH_HOST".into(), "MY_TOKEN".into(), "GITHUB_TOKEN".into()],
            trusted: false,
        }
    }

    #[test]
    fn reads_who_a_tool_says_is_signed_in() {
        assert_eq!(
            identity_from("Logged in to github.com account octo (keyring)").as_deref(),
            Some("octo")
        );
        assert_eq!(
            identity_from("\u{1b}[32m✓\u{1b}[0m Logged in as Octo."),
            Some("Octo".into())
        );
        assert_eq!(identity_from("You are not logged in as anyone"), None);
        assert_eq!(identity_from("{\"token\":\"x\"}"), None);
    }

    #[test]
    fn says_how_to_sign_in_and_install() {
        assert_eq!(
            sign_in_command(&json!({ "probe": { "command": "gh", "args": ["auth", "status"] } })),
            "gh auth login"
        );
        assert_eq!(
            sign_in_command(&json!({ "probe": { "command": "az" } })),
            "az"
        );
        assert!(install_hint("other").contains("`other`"));
    }

    #[test]
    fn borrows_only_what_was_declared_and_is_not_a_credential() {
        assert_eq!(borrowable_names(&cli()), ["GH_HOST", "MY_TOKEN"]);
        let trusted = Source {
            trusted: true,
            ..cli()
        };
        assert_eq!(
            borrowable_names(&trusted),
            ["GH_HOST", "MY_TOKEN", "GITHUB_TOKEN"]
        );
    }

    #[test]
    fn probes_a_cli_rung_and_hands_over_its_token() {
        let missing = Fake::default();
        let report = probe(&cli(), &missing);
        assert_eq!(report["ok"], false);
        assert!(report["installHint"].as_str().is_some());

        let signed_out = Fake {
            installed: true,
            ..Fake::default()
        };
        assert_eq!(
            probe(&cli(), &signed_out)["message"],
            "Sign in by running `gh auth login` in your terminal."
        );

        let signed_in = Fake {
            installed: true,
            answer: Some(Ok(("tok123\n".into(), "Logged in as octo".into()))),
            ..Fake::default()
        };
        assert_eq!(
            probe(&cli(), &signed_in),
            json!({ "ok": true, "identity": "octo" })
        );
        let secrets = borrowed_secrets(&cli(), &signed_in);
        assert_eq!(
            secrets,
            [
                ("GH_HOST".into(), "github.com".into()),
                ("MY_TOKEN".into(), "tok123".into())
            ]
        );
        assert_eq!(
            signed_in.ran.lock().unwrap().last().unwrap(),
            &["auth", "token"]
        );

        assert_eq!(probe(&Source::default(), &missing), json!({ "ok": null }));
        let none = Source {
            auth: Some(json!({ "rung": "none" })),
            ..Source::default()
        };
        assert_eq!(probe(&none, &missing)["ok"], true);
    }
}

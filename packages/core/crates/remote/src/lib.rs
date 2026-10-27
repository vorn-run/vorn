//! Commands run on a remote host over `ssh`: in batch mode, trusting only
//! host keys already known (a new one is accepted where a person sees it, in
//! a terminal), sharing one connection per host for a minute through a folder
//! only this user can reach, with the person's key file and options.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub mod options;

pub use options::Refused;

/// Programs that log in to another machine from a terminal, or attach it to
/// a terminal multiplexer whose panes may be anywhere. A session running one
/// of them, at any depth, is not treated as local.
pub const CLIENTS: &[&str] = &[
    "ssh",
    "mosh",
    "mosh-client",
    "et",
    "autossh",
    "telnet",
    "rsh",
    "rlogin",
    "dbclient",
    "tmux",
    "tmate",
    "screen",
    "zellij",
    "dtach",
    "abduco",
    "dvtm",
    "byobu",
];

/// A remote host as the configuration keeps it (`RemoteHost`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub hostname: String,
    pub user: String,
    #[serde(default = "default_port")]
    pub port: f64,
    #[serde(default)]
    pub ssh_key_path: Option<String>,
    #[serde(default)]
    pub ssh_options: Option<String>,
}

fn default_port() -> f64 {
    22.0
}

/// The arguments before the remote command: options, port, key, the
/// person's options, `--` and `user@host`; refused when the person's options
/// or the destination are not ones [`options`] allows.
pub fn args(
    host: &Host,
    connect_timeout: u32,
    multiplex_dir: Option<&str>,
) -> Result<Vec<String>, Refused> {
    options::check_destination(&host.user, &host.hostname)?;
    let mut args = vec![
        "-o".to_owned(),
        format!("ConnectTimeout={connect_timeout}"),
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        "-o".to_owned(),
        "StrictHostKeyChecking=yes".to_owned(),
    ];
    if let Some(dir) = multiplex_dir {
        args.extend([
            "-o".to_owned(),
            "ControlMaster=auto".to_owned(),
            "-o".to_owned(),
            format!("ControlPath={dir}/%C"),
            "-o".to_owned(),
            "ControlPersist=60".to_owned(),
        ]);
    }
    if host.port != 22.0 {
        args.extend(["-p".to_owned(), options::port(host.port)?.to_string()]);
    }
    if let Some(key) = host.ssh_key_path.as_deref().filter(|k| !k.is_empty()) {
        options::check_key_path(key)?;
        args.extend(["-i".to_owned(), key.to_owned()]);
    }
    if let Some(text) = &host.ssh_options {
        args.extend(options::parse(text)?);
    }
    args.push("--".to_owned());
    args.push(destination(host));
    Ok(args)
}

/// `user@host`, or the bare host when no user is set.
fn destination(host: &Host) -> String {
    if host.user.is_empty() {
        host.hostname.clone()
    } else {
        format!("{}@{}", host.user, host.hostname)
    }
}

/// The folder connections are shared through, once [`share_connections_in`] made it.
static MULTIPLEX: OnceLock<String> = OnceLock::new();

/// The longest folder a control socket may sit in: a socket's path holds at
/// most 103 bytes on macOS (108 with its NUL on Linux); `%C` adds a slash and
/// 40 characters, and ssh a 17-character suffix while it sets one up.
const MAX_MULTIPLEX_DIR: usize = 103 - 1 - 40 - 17;

/// Shares connections through `<data_dir>/ssh`, made (or made again) for
/// this user alone. Without it, or where the path would not make a usable
/// socket, each command logs in on its own.
pub fn share_connections_in(data_dir: &Path) -> std::io::Result<()> {
    let dir = data_dir.join("ssh");
    let text = private_dir(&dir)?.to_string_lossy().into_owned();
    if usable_multiplex_dir(&text) {
        let _ = MULTIPLEX.set(text);
    }
    Ok(())
}

/// `dir`, created 0700, or set to 0700 when it was there already.
fn private_dir(dir: &Path) -> std::io::Result<PathBuf> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    builder.create(dir)?;
    Ok(dir.to_owned())
}

/// Whether ssh can take `dir` whole in `ControlPath`: short enough for a
/// socket, and with nothing ssh would read as a separator or a token.
fn usable_multiplex_dir(dir: &str) -> bool {
    !dir.is_empty()
        && dir.len() <= MAX_MULTIPLEX_DIR
        && !dir.contains(|c: char| c.is_whitespace() || matches!(c, '%' | '"' | '\\'))
}

/// Where connections are shared, if [`share_connections_in`] set a folder;
/// none on Windows, whose ssh has no multiplexing.
pub fn multiplex_dir() -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    MULTIPLEX.get().cloned()
}

/// What a finished `ssh` printed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// `None` when it was killed for running too long.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `ssh <args> <command>` with `env`, killed after `timeout`.
pub fn run(
    ssh: &str,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
) -> std::io::Result<Output> {
    run_with(ssh, args, env, None, timeout)
}

/// [`run`], writing `input` to the program's stdin first.
pub fn run_with(
    ssh: &str,
    args: &[String],
    env: &[(String, String)],
    input: Option<&[u8]>,
    timeout: Duration,
) -> std::io::Result<Output> {
    let mut child = vorn_spawn::command(ssh)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        let bytes = bytes.to_vec();
        // Written beside the readers, so a program that answers before it reads all never blocks.
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = stdin.write_all(&bytes);
        });
    }
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut text);
            }
            String::from_utf8_lossy(&text).into_owned()
        })
    };
    let out = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Output {
        code,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

/// `text` as one word to a POSIX shell (`shellEscape(text, 'posix')`).
pub fn quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-./:=@+,%".contains(&b));
    if plain {
        return text.to_owned();
    }
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// How a host is logged in to for one command: ssh's arguments
/// through `user@host`, and the environment ssh runs with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Login {
    /// The login for `host`, with a ten-second connect timeout; refused as [`args`] is.
    pub fn new(host: &Host, env: Vec<(String, String)>) -> Result<Login, Refused> {
        Ok(Login {
            args: args(host, 10, multiplex_dir().as_deref())?,
            env,
        })
    }

    /// Runs `command` in a shell on the host: its output, or why it failed
    /// (`Command failed: ...`).
    pub fn exec(
        &self,
        command: &str,
        input: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<String, String> {
        let mut argv = self.args.clone();
        argv.push(command.to_owned());
        let out = run_with("ssh", &argv, &self.env, input, timeout)
            .map_err(|e| format!("spawn ssh {e}"))?;
        match out.code {
            Some(0) => Ok(out.stdout),
            code => {
                let line = format!("ssh {}", argv.join(" "));
                let why = match code {
                    Some(_) => out.stderr.trim_end().to_owned(),
                    None => "timed out".to_owned(),
                };
                Err(format!("Command failed: {line}\n{why}")
                    .trim_end()
                    .to_owned())
            }
        }
    }
}

/// `ssh:testConnection`'s answer (`SshTestResult`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tested {
    pub success: bool,
    pub message: String,
    pub duration_ms: u64,
}

const OK_MARKER: &str = "__VORN_OK__";

/// What a test says of a host whose key this machine has not seen: Vorn
/// never accepts one on its own, so the person checks it where ssh shows it.
pub const UNKNOWN_HOST_KEY: &str = "This host's key is not trusted yet. Connect to it once from a terminal with ssh to check and accept its key, then test again.";

/// Logs in to `host` and runs `echo __VORN_OK__`.
pub fn test(host: &Host, env: &[(String, String)]) -> Tested {
    let started = Instant::now();
    let mut argv = match args(host, 5, multiplex_dir().as_deref()) {
        Ok(argv) => argv,
        Err(refused) => {
            return Tested {
                success: false,
                message: refused.to_string(),
                duration_ms: 0,
            }
        }
    };
    argv.extend(["echo".to_owned(), OK_MARKER.to_owned()]);
    let ran = run("ssh", &argv, env, Duration::from_secs(10));
    let duration_ms = started.elapsed().as_millis() as u64;
    tested(ran, duration_ms)
}

fn tested(ran: std::io::Result<Output>, duration_ms: u64) -> Tested {
    let (ok, stderr, fallback) = match &ran {
        Ok(out) => (
            out.code == Some(0) && out.stdout.contains(OK_MARKER),
            out.stderr.as_str(),
            match out.code {
                Some(code) => format!("Command failed with exit code {code}"),
                None => "Command timed out".to_owned(),
            },
        ),
        Err(e) => (false, "", e.to_string()),
    };
    if ok {
        return Tested {
            success: true,
            message: format!("Connected in {duration_ms}ms"),
            duration_ms,
        };
    }
    // ssh's warnings, such as a host newly added to known_hosts, are not the failure.
    let clean = stderr
        .split('\n')
        .filter(|l| !l.starts_with("Warning:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();
    let mut message = if clean.is_empty() { fallback } else { clean };
    if message.contains("host key is known for") {
        message = UNKNOWN_HOST_KEY.to_owned();
    } else if message.contains("Host key verification failed") {
        message =
            "Host key changed — remove old entry from known_hosts or verify the server".to_owned();
    } else if message.contains("Permission denied") {
        message = "Permission denied — check username and authentication method".to_owned();
    }
    Tested {
        success: false,
        message,
        duration_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host {
            hostname: "box".into(),
            user: "me".into(),
            port: 2222.0,
            ssh_key_path: Some("/k".into()),
            ssh_options: Some(" -o  ServerAliveInterval=30 ".into()),
        }
    }

    #[test]
    fn builds_the_ssh_arguments() {
        assert_eq!(
            args(&host(), 5, Some("/t")).unwrap().join(" "),
            "-o ConnectTimeout=5 -o BatchMode=yes -o StrictHostKeyChecking=yes \
             -o ControlMaster=auto -o ControlPath=/t/%C -o ControlPersist=60 \
             -p 2222 -i /k -o ServerAliveInterval=30 -- me@box"
        );
        let plain: Host = serde_json::from_value(serde_json::json!({
            "id": "h", "label": "H", "hostname": "box", "user": "me", "port": 22
        }))
        .unwrap();
        let argv = args(&plain, 10, None).unwrap();
        assert_eq!(argv[argv.len() - 2..], ["--", "me@box"]);
        assert!(!argv.contains(&"-p".to_owned()));
    }

    #[test]
    fn refuses_a_host_whose_options_or_names_are_not_allowed() {
        let with = |options: &str| Host {
            ssh_options: Some(options.into()),
            ..host()
        };
        assert_eq!(
            args(&with("-o ProxyCommand=nc"), 5, None)
                .unwrap_err()
                .to_string(),
            "ssh option ProxyCommand is not allowed"
        );
        let dashed = Host {
            hostname: "-oProxyCommand=x".into(),
            ..host()
        };
        assert!(Login::new(&dashed, Vec::new()).is_err());
        let refused = test(&with("-o LocalCommand=x"), &[]);
        assert!(!refused.success);
        assert_eq!(refused.message, "ssh option LocalCommand is not allowed");
    }

    #[test]
    fn says_why_a_login_failed() {
        let out = |code, stdout: &str, stderr: &str| {
            Ok(Output {
                code,
                stdout: stdout.into(),
                stderr: stderr.into(),
            })
        };
        assert_eq!(
            tested(out(Some(0), "__VORN_OK__\n", ""), 12),
            Tested {
                success: true,
                message: "Connected in 12ms".into(),
                duration_ms: 12
            }
        );
        let denied = tested(
            out(
                Some(255),
                "",
                "Warning: added\nme@box: Permission denied (publickey).",
            ),
            1,
        );
        assert_eq!(
            denied.message,
            "Permission denied — check username and authentication method"
        );
        let key = tested(out(Some(255), "", "Host key verification failed."), 1);
        assert!(key.message.starts_with("Host key changed"));
        let unknown = tested(
            out(
                Some(255),
                "",
                "No ED25519 host key is known for box and you have requested strict checking.\nHost key verification failed.",
            ),
            1,
        );
        assert_eq!(unknown.message, UNKNOWN_HOST_KEY);
        assert_eq!(
            tested(out(Some(255), "", "Warning: only"), 1).message,
            "Command failed with exit code 255"
        );
        assert_eq!(tested(out(None, "", ""), 1).message, "Command timed out");
        let missing = tested(Err(std::io::Error::other("spawn ssh ENOENT")), 1);
        assert_eq!(
            (missing.success, missing.message.as_str()),
            (false, "spawn ssh ENOENT")
        );
    }

    #[test]
    fn shares_connections_only_through_a_folder_ssh_can_use() {
        assert!(usable_multiplex_dir("/Users/me/.vorn/ssh"));
        assert!(!usable_multiplex_dir("/Users/me/Application Support/ssh"));
        assert!(!usable_multiplex_dir("/tmp/100%/ssh"));
        assert!(usable_multiplex_dir(&format!("/{}", "a".repeat(44))));
        assert!(!usable_multiplex_dir(&format!("/{}", "a".repeat(45))));
        assert!(!usable_multiplex_dir(""));
    }

    #[cfg(unix)]
    #[test]
    fn the_shared_folder_is_this_users_alone() {
        use std::os::unix::fs::PermissionsExt;
        let data = tempfile::tempdir().unwrap();
        let dir = data.path().join("ssh");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn quotes_a_word_for_a_posix_shell() {
        assert_eq!(quote("/srv/app-1"), "/srv/app-1");
        assert_eq!(quote("it's here"), "'it'\\''s here'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("$HOME"), "'$HOME'");
    }

    #[cfg(unix)]
    #[test]
    fn hands_a_program_its_input() {
        let env: Vec<(String, String)> = std::env::vars().collect();
        let done = run_with(
            "sh",
            &["-c".into(), "cat".into()],
            &env,
            Some(b"in"),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(done.stdout, "in");
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_program_and_kills_one_that_runs_too_long() {
        let env: Vec<(String, String)> = std::env::vars().collect();
        let done = run(
            "sh",
            &["-c".into(), "echo out; echo err >&2; exit 3".into()],
            &env,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(
            (done.code, done.stdout.as_str(), done.stderr.as_str()),
            (Some(3), "out\n", "err\n")
        );
        let slow = run(
            "sh",
            &["-c".into(), "sleep 5".into()],
            &env,
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(slow.code, None);
    }
}

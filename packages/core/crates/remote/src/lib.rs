//! Commands run on a remote host over `ssh`, as the server's `buildSshArgs`
//! and `sshExec` run them: in batch mode, accepting a new host key, sharing
//! one connection per host for a minute where ssh can, with the person's key
//! file and options.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

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
/// person's options and `user@host`.
pub fn args(host: &Host, connect_timeout: u32, multiplex_dir: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-o".to_owned(),
        format!("ConnectTimeout={connect_timeout}"),
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        "-o".to_owned(),
        "StrictHostKeyChecking=accept-new".to_owned(),
    ];
    if let Some(dir) = multiplex_dir {
        args.extend([
            "-o".to_owned(),
            "ControlMaster=auto".to_owned(),
            "-o".to_owned(),
            format!("ControlPath={dir}/vorn-ssh-%h-%p"),
            "-o".to_owned(),
            "ControlPersist=60".to_owned(),
        ]);
    }
    if host.port != 22.0 {
        args.extend(["-p".to_owned(), format!("{}", host.port)]);
    }
    if let Some(key) = host.ssh_key_path.as_deref().filter(|k| !k.is_empty()) {
        args.extend(["-i".to_owned(), key.to_owned()]);
    }
    if let Some(options) = &host.ssh_options {
        args.extend(options.split_whitespace().map(str::to_owned));
    }
    args.push(format!("{}@{}", host.user, host.hostname));
    args
}

/// Where connections are shared: `$TMPDIR` or `/tmp`; none on Windows.
pub fn multiplex_dir() -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    Some(
        std::env::var("TMPDIR")
            .ok()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "/tmp".to_owned()),
    )
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
    let mut child = Command::new(ssh)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
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

/// `ssh:testConnection`'s answer (`SshTestResult`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tested {
    pub success: bool,
    pub message: String,
    pub duration_ms: u64,
}

const OK_MARKER: &str = "__VORN_OK__";

/// Logs in to `host` and runs `echo __VORN_OK__`, as `testSshConnection` does.
pub fn test(host: &Host, env: &[(String, String)]) -> Tested {
    let started = Instant::now();
    let mut argv = args(host, 5, multiplex_dir().as_deref());
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
    if message.contains("Host key verification failed") {
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
            ssh_options: Some(" -o  Foo=bar ".into()),
        }
    }

    #[test]
    fn builds_the_arguments_the_server_does() {
        assert_eq!(
            args(&host(), 5, Some("/t")).join(" "),
            "-o ConnectTimeout=5 -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
             -o ControlMaster=auto -o ControlPath=/t/vorn-ssh-%h-%p -o ControlPersist=60 \
             -p 2222 -i /k -o Foo=bar me@box"
        );
        let plain: Host = serde_json::from_value(serde_json::json!({
            "id": "h", "label": "H", "hostname": "box", "user": "me", "port": 22
        }))
        .unwrap();
        assert_eq!(args(&plain, 10, None).last().unwrap(), "me@box");
        assert!(!args(&plain, 10, None).contains(&"-p".to_owned()));
    }

    #[test]
    fn says_why_a_login_failed_in_the_server_s_words() {
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

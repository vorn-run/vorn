//! A session on a remote host, as the server's `createRemotePty` starts it:
//! a local shell, into which the `ssh` line is typed ([`ssh_line`]); the
//! password typed when ssh asks for it; and, once the remote shell has said
//! it is up, the agent's launch line run in the project there
//! ([`remote_command`]). [`Login`] reads the session's output and says when
//! to do each.
//!
//! The ready marker is typed split by quotes ([`ssh_line`]) on a POSIX
//! shell, so the local shell's echo of the line does not count as the
//! remote one printing it; ssh is given the same argument either way.

use std::time::Duration;

use super::{Platform, Quoting};
use crate::js;

/// How the host is logged in to (`AuthMethod`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auth<'a> {
    /// ssh-agent: no flags.
    Agent,
    /// A key file on this machine, when one is named.
    KeyFile(Option<&'a str>),
    /// A key kept encrypted in Vorn, written to this file for the login;
    /// `None` when it could not be decrypted, which falls back to the agent.
    StoredKey(Option<&'a str>),
    /// A password, typed when ssh asks for it.
    Password,
}

impl<'a> Auth<'a> {
    /// The auth method as stored (`agent` when none is), with the key file
    /// it uses. An unknown method passes no flags, as the server's does.
    pub fn from_stored(
        method: Option<&str>,
        key_path: Option<&'a str>,
        stored_key: Option<&'a str>,
    ) -> Auth<'a> {
        match method.unwrap_or("agent") {
            "key-file" => Auth::KeyFile(key_path.filter(|p| !p.is_empty())),
            "key-stored" => Auth::StoredKey(stored_key),
            "password" => Auth::Password,
            _ => Auth::Agent,
        }
    }
}

/// The host a session logs in to (`RemoteHost`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target<'a> {
    pub hostname: &'a str,
    pub user: &'a str,
    pub port: f64,
    pub auth: Auth<'a>,
    /// Extra ssh options as the person typed them, split on whitespace.
    pub options: Option<&'a str>,
}

/// The marker the remote shell prints once it is up, for session `id`.
pub fn marker(id: &str) -> String {
    let short: String = id.chars().take(8).collect();
    format!("__VORN_READY_{short}__")
}

/// The line typed into the local shell to log in: ssh with a terminal, the
/// port, the key, the person's options and the host, running `echo marker`
/// and then a login shell there. Not quoted word by word: the options are
/// read by the local shell as the person wrote them.
pub fn ssh_line(target: &Target<'_>, marker: &str, local: Platform) -> String {
    let mut parts: Vec<String> = vec!["ssh".into(), "-t".into()];
    if target.port != 22.0 {
        parts.push("-p".into());
        parts.push(format!("{}", target.port));
    }
    match target.auth {
        Auth::KeyFile(Some(path)) | Auth::StoredKey(Some(path)) => {
            parts.push("-i".into());
            parts.push(path.to_owned());
        }
        Auth::Password => {
            parts.push("-o".into());
            parts.push("PreferredAuthentications=password".into());
            parts.push("-o".into());
            parts.push("PubkeyAuthentication=no".into());
        }
        Auth::Agent | Auth::KeyFile(None) | Auth::StoredKey(None) => {}
    }
    if let Some(options) = target.options {
        parts.extend(
            options
                .split(js::is_space)
                .filter(|o| !o.is_empty())
                .map(str::to_owned),
        );
    }
    parts.push(format!("{}@{}", target.user, target.hostname));
    let shown = match (local, marker.strip_prefix("__VORN_READY_")) {
        // Adjacent quoted words are one word: ssh gets the marker whole.
        (Platform::Posix, Some(rest)) => format!("__VORN_READY_'{rest}'"),
        _ => marker.to_owned(),
    };
    parts.push(format!("'echo {shown} && exec $SHELL -l'"));
    parts.join(" ")
}

/// What runs in the remote shell once it is up: into the project, then the
/// agent (`cd <project> && <launch line>`).
pub fn remote_command(project_path: &str, launch_line: &str) -> String {
    format!("cd {} && {launch_line}", Quoting::Posix.quote(project_path))
}

/// How long after the session starts ssh may still ask for the password.
pub const PASSWORD_FOR: Duration = Duration::from_secs(15);
/// When the command is typed anyway if the marker never shows.
pub const FALLBACK_AFTER: Duration = Duration::from_secs(8);
/// How long the output is read for the marker or a failed login.
pub const MARKER_FOR: Duration = Duration::from_secs(10);
/// How long after the prompt the password is typed.
pub const PASSWORD_DELAY: Duration = Duration::from_millis(50);
/// How long after the marker the command is typed: the login shell
/// finishes starting.
pub const COMMAND_DELAY: Duration = Duration::from_millis(200);

/// What ssh prints when it cannot log in.
const FAILURES: [&str; 7] = [
    "Permission denied",
    "Connection refused",
    "Connection timed out",
    "Could not resolve hostname",
    "No route to host",
    "Connection closed",
    "Host key verification failed",
];

/// What to do next in a login.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginStep {
    /// Type the password, after [`PASSWORD_DELAY`].
    Password,
    /// The remote shell is up: type the command after [`COMMAND_DELAY`].
    Connected,
    /// The marker never showed: type the command now.
    Fallback,
    /// ssh said it could not log in, in these words. Nothing is typed
    /// unless the marker shows after all.
    Failed(&'static str),
}

/// A login read from the session's output, chunk by chunk.
#[derive(Debug)]
pub struct Login {
    marker: String,
    wants_password: bool,
    password_sent: bool,
    connected: bool,
    failed: bool,
    /// Everything read until the marker, as the server keeps it.
    seen: String,
}

impl Login {
    /// A login waiting for `marker`, typing a password when `wants_password`.
    pub fn new(marker: String, wants_password: bool) -> Login {
        Login {
            marker,
            wants_password,
            password_sent: false,
            connected: false,
            failed: false,
            seen: String::new(),
        }
    }

    /// Whether the command has been typed or is due to be.
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// Whether there is nothing left to read for, `elapsed` after the start.
    pub fn done(&self, elapsed: Duration) -> bool {
        let password = self.wants_password && !self.password_sent && elapsed < PASSWORD_FOR;
        let marker = !self.connected && elapsed < MARKER_FOR;
        !password && !marker
    }

    /// Reads `chunk`, printed `elapsed` after the start.
    pub fn feed(&mut self, chunk: &str, elapsed: Duration) -> Vec<LoginStep> {
        let mut steps = Vec::new();
        if self.wants_password
            && !self.password_sent
            && elapsed < PASSWORD_FOR
            && asks_password(chunk)
        {
            self.password_sent = true;
            steps.push(LoginStep::Password);
        }
        if self.connected || elapsed >= MARKER_FOR {
            return steps;
        }
        self.seen.push_str(chunk);
        if self.seen.contains(&self.marker) {
            self.connected = true;
            self.seen = String::new();
            steps.push(LoginStep::Connected);
        } else if !self.failed {
            if let Some(why) = FAILURES.into_iter().find(|f| self.seen.contains(f)) {
                self.failed = true;
                steps.push(LoginStep::Failed(why));
            }
        }
        steps
    }

    /// Whether the command is still to be typed at [`FALLBACK_AFTER`] if nothing else happens.
    pub fn awaits_fallback(&self) -> bool {
        !self.connected && !self.failed
    }

    /// What is due `elapsed` after the start with nothing printed: the
    /// command, once, if the marker never showed and ssh did not fail.
    pub fn due(&mut self, elapsed: Duration) -> Option<LoginStep> {
        if !self.awaits_fallback() || elapsed < FALLBACK_AFTER {
            return None;
        }
        self.connected = true;
        Some(LoginStep::Fallback)
    }
}

/// `/[Pp]ass(word|phrase)[^:]*:\s*$/`: a chunk ending in a password prompt.
fn asks_password(chunk: &str) -> bool {
    chunk.match_indices("ass").any(|(at, _)| {
        let before = chunk[..at].chars().next_back();
        let after = &chunk[at + 3..];
        let Some(rest) = matches!(before, Some('P' | 'p'))
            .then(|| {
                after
                    .strip_prefix("word")
                    .or_else(|| after.strip_prefix("phrase"))
            })
            .flatten()
        else {
            return false;
        };
        match rest.split_once(':') {
            Some((_, tail)) => tail.chars().all(js::is_space),
            None => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(auth: Auth<'_>) -> Target<'_> {
        Target {
            hostname: "box.example",
            user: "me",
            port: 22.0,
            auth,
            options: None,
        }
    }

    #[test]
    fn types_ssh_as_the_server_does_with_the_marker_split_by_quotes() {
        let m = marker("0123456789abcdef");
        assert_eq!(m, "__VORN_READY_01234567__");
        assert_eq!(
            ssh_line(&target(Auth::Agent), &m, Platform::Posix),
            "ssh -t me@box.example 'echo __VORN_READY_'01234567__' && exec $SHELL -l'"
        );
        assert!(!ssh_line(&target(Auth::Agent), &m, Platform::Posix).contains(&m));
        // Elsewhere the line is the server's, word for word.
        assert_eq!(
            ssh_line(&target(Auth::Agent), &m, Platform::Windows),
            "ssh -t me@box.example 'echo __VORN_READY_01234567__ && exec $SHELL -l'"
        );
    }

    #[test]
    fn passes_the_port_key_and_options_for_each_way_of_logging_in() {
        let line = |t: Target<'_>| ssh_line(&t, "M", Platform::Windows);
        let with = Target {
            port: 2222.0,
            options: Some("  -o\tProxyJump=hop   -A "),
            ..target(Auth::KeyFile(Some("/k/id")))
        };
        assert_eq!(
            line(with),
            "ssh -t -p 2222 -i /k/id -o ProxyJump=hop -A me@box.example 'echo M && exec $SHELL -l'"
        );
        assert_eq!(
            line(target(Auth::StoredKey(Some("/tmp/vorn-key-1")))),
            "ssh -t -i /tmp/vorn-key-1 me@box.example 'echo M && exec $SHELL -l'"
        );
        // A stored key that could not be decrypted, or a key file not named: the agent.
        for auth in [Auth::StoredKey(None), Auth::KeyFile(None)] {
            assert_eq!(
                line(target(auth)),
                "ssh -t me@box.example 'echo M && exec $SHELL -l'"
            );
        }
        assert_eq!(
            line(target(Auth::Password)),
            "ssh -t -o PreferredAuthentications=password -o PubkeyAuthentication=no me@box.example 'echo M && exec $SHELL -l'"
        );
    }

    #[test]
    fn reads_the_stored_auth_method() {
        assert_eq!(Auth::from_stored(None, Some("/k"), None), Auth::Agent);
        assert_eq!(
            Auth::from_stored(Some("key-file"), Some(""), None),
            Auth::KeyFile(None)
        );
        assert_eq!(
            Auth::from_stored(Some("key-stored"), None, Some("/t")),
            Auth::StoredKey(Some("/t"))
        );
        assert_eq!(
            Auth::from_stored(Some("password"), None, None),
            Auth::Password
        );
        assert_eq!(Auth::from_stored(Some("other"), None, None), Auth::Agent);
    }

    #[test]
    fn runs_the_agent_in_the_quoted_project() {
        assert_eq!(
            remote_command("/srv/app", "claude"),
            "cd /srv/app && claude"
        );
        assert_eq!(
            remote_command("/srv/it's here", "codex --x"),
            "cd '/srv/it'\\''s here' && codex --x"
        );
    }

    #[test]
    fn knows_a_password_prompt_as_the_server_does() {
        for yes in [
            "me@box's password: ",
            "Password:",
            "Enter passphrase for key '/k/id': \r\n",
        ] {
            assert!(asks_password(yes), "{yes:?}");
        }
        for no in [
            "Password: hunter",
            "password",
            "PASSWORD:",
            "Last login: password changed",
            "",
        ] {
            assert!(!asks_password(no), "{no:?}");
        }
    }

    #[test]
    fn types_the_password_once_then_the_command_at_the_marker() {
        let mut login = Login::new(marker("abcdefgh-1"), true);
        let t = Duration::from_millis(500);
        assert_eq!(login.feed("me@box's password: ", t), [LoginStep::Password]);
        assert_eq!(login.feed("Password: ", t), []);
        // The echo of the typed line is not the marker.
        let echo = ssh_line(
            &target(Auth::Password),
            &marker("abcdefgh-1"),
            Platform::Posix,
        );
        assert_eq!(login.feed(&echo, t), []);
        assert_eq!(login.feed("__VORN_RE", t), []);
        assert_eq!(
            login.feed("ADY_abcdefgh__\r\n$ ", t),
            [LoginStep::Connected]
        );
        assert!(login.connected());
        assert_eq!(login.feed("__VORN_READY_abcdefgh__", t), []);
        assert_eq!(login.due(FALLBACK_AFTER), None);
        assert!(login.done(t));
    }

    #[test]
    fn falls_back_to_typing_the_command_unless_the_login_failed() {
        let mut login = Login::new(marker("id"), false);
        assert_eq!(login.due(FALLBACK_AFTER - Duration::from_millis(1)), None);
        assert_eq!(login.due(FALLBACK_AFTER), Some(LoginStep::Fallback));
        assert_eq!(login.due(FALLBACK_AFTER), None);

        let mut failed = Login::new(marker("id"), false);
        let t = Duration::from_secs(1);
        assert_eq!(
            failed.feed("ssh: connect to host box port 22: Connection ref", t),
            []
        );
        assert_eq!(
            failed.feed("used\r\n", t),
            [LoginStep::Failed("Connection refused")]
        );
        assert_eq!(failed.feed("Connection refused", t), []);
        assert_eq!(failed.due(FALLBACK_AFTER), None);
        assert!(!failed.awaits_fallback());
        assert!(!failed.done(t));
        assert!(failed.done(MARKER_FOR));
    }

    #[test]
    fn stops_reading_past_its_deadlines() {
        let mut login = Login::new(marker("id"), true);
        assert_eq!(login.feed("__VORN_READY_id__", MARKER_FOR), []);
        assert_eq!(login.feed("Password:", PASSWORD_FOR), []);
        assert!(login.done(PASSWORD_FOR));
        assert!(!Login::new(marker("id"), true).done(MARKER_FOR));
    }
}

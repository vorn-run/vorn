//! The environment vornd runs git, `which`, IDEs and agents with.
//!
//! The "safe" environment: the login shell's once it has answered
//! (`$SHELL -ilc`, read between sentinels), on Windows the registry's PATH
//! for new processes, else vornd's own; its PATH ends in the places agent
//! CLIs install to; and it drops credential-shaped variables and agent CLI
//! markers (`vorn_agents::launch::env`). An app started from the Dock or
//! Explorer has only the system PATH, so without this an installed agent
//! would not be found.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tracing::{info, warn};
// Which names are dropped, and how PATH is searched, are the launch
// builders' rules, shared with them.
use vorn_agents::launch::env::{launch_env, safe_env};
use vorn_agents::launch::{find_on_path as find_launchable, Platform};

/// How long the login shell has to print its environment.
const SHELL_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a shell that did not answer is left alone before it is asked again.
const SHELL_RETRY: Duration = Duration::from_secs(30);

/// The most the shell may print; past it the answer is not kept.
const SHELL_MAX_OUTPUT: usize = 4 * 1024 * 1024;

/// Brackets the environment in what the shell prints, so its startup files' output is not read as variables.
const SENTINEL: &str = "__VORN_ENV_5e1f__";

/// An environment, as name and value pairs.
pub type Env = Vec<(String, String)>;

#[derive(Debug)]
struct Shell {
    /// The last answer; kept while a refresh asks again.
    answer: Option<Arc<Env>>,
    asking: bool,
    /// When a failed ask may be retried.
    retry_at: Instant,
}

/// The safe environment, and the login shell's answer once it has one.
#[derive(Debug)]
pub struct SafeEnv {
    shell: Mutex<Shell>,
    answered: Condvar,
}

impl Default for SafeEnv {
    fn default() -> Self {
        SafeEnv {
            shell: Mutex::new(Shell {
                answer: None,
                asking: false,
                retry_at: Instant::now(),
            }),
            answered: Condvar::new(),
        }
    }
}

impl SafeEnv {
    pub fn new() -> Arc<SafeEnv> {
        Arc::new(SafeEnv::default())
    }

    fn shell(&self) -> MutexGuard<'_, Shell> {
        self.shell.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The environment to run a program with: the shell's once it answered,
    /// this process's until then, filtered either way. Asks the shell again
    /// when it is time to.
    pub fn get(self: &Arc<Self>) -> Env {
        safe_env(self.source())
    }

    /// The environment an agent runs with: [`SafeEnv::get`]
    /// but for the names the person passed through (`envPassthrough`), and
    /// with `VORN_DATA_DIR` naming vornd's data directory.
    pub fn launch(self: &Arc<Self>, passthrough: &[String], data_dir: Option<&Path>) -> Env {
        let dir = data_dir.map(|d| d.to_string_lossy().into_owned());
        launch_env(self.source(), passthrough, dir.as_deref())
    }

    /// The shell's environment once it answered, this process's until then,
    /// unfiltered, its PATH ending in the places agent CLIs install to.
    fn source(self: &Arc<Self>) -> Env {
        let (answered, ask) = {
            let mut shell = self.shell();
            let ask = shell.answer.is_none() && !shell.asking && Instant::now() >= shell.retry_at;
            shell.asking |= ask;
            (shell.answer.clone(), ask)
        };
        // Asked with the lock released.
        if ask {
            self.ask();
        }
        let env = match answered {
            Some(env) => env.as_ref().clone(),
            None => std::env::vars().collect(),
        };
        with_fallback_path(env)
    }

    /// Whether the login shell has answered.
    pub fn resolved(&self) -> bool {
        self.shell().answer.is_some()
    }

    /// Starts asking the login shell, once.
    pub fn prime(self: &Arc<Self>) {
        let mut shell = self.shell();
        if shell.answer.is_none() && !shell.asking {
            shell.asking = true;
            drop(shell);
            self.ask();
        }
    }

    /// Asks the login shell again, as after installing an agent; the last answer serves until then.
    pub fn refresh(self: &Arc<Self>) {
        let mut shell = self.shell();
        if !shell.asking {
            shell.asking = true;
            drop(shell);
            self.ask();
        }
    }

    /// Waits, at most `max`, while the shell is being asked.
    pub fn settle(&self, max: Duration) {
        let shell = self.shell();
        let _ = self
            .answered
            .wait_timeout_while(shell, max, |s| s.asking)
            .map_err(|e| e.into_inner());
    }

    /// Asks on a thread of its own; nothing waits for it.
    fn ask(self: &Arc<Self>) {
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = if cfg!(windows) {
                Ok(registry_env())
            } else {
                login_shell_env()
            };
            let mut shell = this.shell();
            match result {
                Ok(env) => {
                    info!(
                        ms = started.elapsed().as_millis() as u64,
                        "login shell environment resolved"
                    );
                    shell.answer = Some(Arc::new(env));
                }
                Err(err) => {
                    warn!(%err, "the login shell did not answer; programs get vornd's PATH and the usual install places until it does");
                    shell.retry_at = Instant::now() + SHELL_RETRY;
                }
            }
            shell.asking = false;
            this.answered.notify_all();
        });
    }

    /// Where `name` is on the safe environment's PATH.
    pub fn find(self: &Arc<Self>, name: &str) -> Option<PathBuf> {
        let env = self.get();
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH" || k == "Path")
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var("PATH").ok())?;
        find_on_path(name, &path)
    }

    /// The git to run: found on PATH, else the bare name.
    pub fn git_bin(self: &Arc<Self>) -> String {
        self.find("git")
            .map_or_else(|| "git".to_owned(), |p| p.to_string_lossy().into_owned())
    }
}

/// The script the login shell runs: the environment, NUL-separated where `env -0` exists, between sentinels.
fn shell_script() -> String {
    format!(
        "printf '%s' {SENTINEL}; command env -0 2>/dev/null || command env; printf '%s' {SENTINEL}"
    )
}

/// `$SHELL -ilc <script>`, with this process's filtered environment.
fn login_shell_env() -> Result<Env, String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_owned());
    shell_env(&shell)
}

/// What `shell -ilc <script>` reports as its environment.
fn shell_env(shell: &str) -> Result<Env, String> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = vorn_spawn::command(shell)
        .args(["-ilc", &shell_script()])
        .env_clear()
        .envs(filter(std::env::vars()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{shell}: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut limited = (&mut stdout).take(SHELL_MAX_OUTPUT as u64 + 1);
        let _ = limited.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + SHELL_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{shell} did not answer within {SHELL_TIMEOUT:?}"));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = reader.join().map_err(|_| "the reader stopped")?;
    if out.len() > SHELL_MAX_OUTPUT {
        return Err(format!(
            "{shell} printed more than {SHELL_MAX_OUTPUT} bytes"
        ));
    }
    // A startup file that fails can set the exit status; the sentinels say whether env ran.
    parse_shell_output(&String::from_utf8_lossy(&out))
        .ok_or_else(|| format!("{shell} printed no environment"))
}

/// The environment between the sentinels of what the shell printed.
fn parse_shell_output(output: &str) -> Option<Env> {
    let start = output.find(SENTINEL)? + SENTINEL.len();
    let end = output.rfind(SENTINEL).filter(|&e| e >= start)?;
    let body = &output[start..end];
    let env = if body.contains('\0') {
        parse_env_records(body.split('\0'))
    } else {
        parse_env_output(body)
    };
    (!env.is_empty()).then_some(env)
}

/// The directories agent CLIs install to, for a PATH that lacks them (an app
/// started from the Dock or Explorer, a shell that did not answer).
fn fallback_dirs(home: &Path, lookup: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        let var = |name: &str| lookup(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        dirs.extend(var("APPDATA").map(|d| d.join("npm")));
        dirs.extend(var("LOCALAPPDATA").map(|d| d.join("pnpm")));
        dirs.extend(var("LOCALAPPDATA").map(|d| d.join("Volta").join("bin")));
        dirs.extend(var("LOCALAPPDATA").map(|d| d.join("Microsoft").join("WinGet").join("Links")));
        dirs.extend(var("NVM_SYMLINK"));
        for sub in [
            ".local\\bin",
            ".bun\\bin",
            ".cargo\\bin",
            "scoop\\shims",
            ".volta\\bin",
        ] {
            dirs.push(home.join(sub));
        }
        return dirs;
    }
    for sub in [
        ".local/bin",
        ".npm-global/bin",
        ".bun/bin",
        ".cargo/bin",
        ".volta/bin",
        ".asdf/shims",
        ".local/share/mise/shims",
        ".local/share/pnpm",
        "Library/pnpm",
        ".deno/bin",
        ".yarn/bin",
        ".local/share/fnm/aliases/default/bin",
        "Library/Application Support/fnm/aliases/default/bin",
    ] {
        dirs.push(home.join(sub));
    }
    dirs.extend(newest_nvm_node(home));
    for d in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/home/linuxbrew/.linuxbrew/bin",
        "/snap/bin",
    ] {
        dirs.push(PathBuf::from(d));
    }
    dirs
}

/// The `bin` of the newest node nvm installed: where its global npm packages are.
fn newest_nvm_node(home: &Path) -> Option<PathBuf> {
    let versions = home.join(".nvm/versions/node");
    let parse = |name: &str| -> Option<Vec<u64>> {
        name.strip_prefix('v')?
            .split('.')
            .map(|p| p.parse().ok())
            .collect()
    };
    std::fs::read_dir(&versions)
        .ok()?
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            Some((parse(&name)?, name))
        })
        .max()
        .map(|(_, name)| versions.join(name).join("bin"))
}

/// `path` with each of `dirs` that exists and is not on it appended.
fn append_dirs(path: &str, dirs: &[PathBuf], sep: char) -> String {
    let mut out = path.to_owned();
    for dir in dirs {
        let Some(d) = dir.to_str() else { continue };
        let on = out.split(sep).any(|p| p.trim_end_matches(['/', '\\']) == d);
        if !on && dir.is_dir() {
            if !out.is_empty() {
                out.push(sep);
            }
            out.push_str(d);
        }
    }
    out
}

/// `env` with its PATH ending in the [`fallback_dirs`] it lacks.
fn with_fallback_path(mut env: Env) -> Env {
    let lookup = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    let home = lookup(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .or_else(|| std::env::home_dir().map(|h| h.to_string_lossy().into_owned()));
    let Some(home) = home.filter(|h| !h.is_empty()) else {
        return env;
    };
    let dirs = fallback_dirs(Path::new(&home), lookup);
    let sep = if cfg!(windows) { ';' } else { ':' };
    match env.iter_mut().find(|(k, _)| k == "PATH" || k == "Path") {
        Some((_, path)) => *path = append_dirs(path, &dirs, sep),
        None => env.push(("PATH".to_owned(), append_dirs("", &dirs, sep))),
    }
    env
}

/// This process's environment with the PATH Windows keeps for new
/// processes: the machine's and the user's, as Explorer would give a
/// program started now, then what this process had.
fn registry_env() -> Env {
    let mut env: Env = std::env::vars().collect();
    let query = |key: &str| -> Option<String> {
        let out = vorn_spawn::command("reg")
            .args(["query", key, "/v", "Path"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| parse_reg_path(&String::from_utf8_lossy(&out.stdout)))
            .flatten()
    };
    let machine = query(r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment");
    let user = query(r"HKCU\Environment");
    let lookup = |name: &str| std::env::var(name).ok();
    let registry: Vec<String> = [machine, user]
        .into_iter()
        .flatten()
        .map(|p| expand_percent(&p, lookup))
        .collect();
    let dirs: Vec<PathBuf> = registry
        .iter()
        .flat_map(|p| p.split(';'))
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .collect();
    match env.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case("PATH")) {
        Some((_, path)) => *path = append_dirs(path, &dirs, ';'),
        None => env.push(("Path".to_owned(), append_dirs("", &dirs, ';'))),
    }
    env
}

/// The value of `Path` in what `reg query <key> /v Path` printed.
fn parse_reg_path(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut parts = line.trim().splitn(3, "    ");
        let (name, kind, value) = (parts.next()?, parts.next()?, parts.next()?);
        (name.eq_ignore_ascii_case("Path") && kind.starts_with("REG_"))
            .then(|| value.trim().to_owned())
    })
}

/// `%NAME%` replaced by `lookup(NAME)`, as `REG_EXPAND_SZ` is; unknown names stay as written.
fn expand_percent(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('%') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('%') {
            Some(close) => {
                let name = &after[..close];
                match lookup(name).filter(|_| !name.is_empty()) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// `NAME=value` lines; a line without `=` past its first character is skipped.
fn parse_env_output(output: &str) -> Env {
    parse_env_records(output.split('\n'))
}

/// `NAME=value` records; one without `=` past its first character is skipped.
fn parse_env_records<'a>(records: impl Iterator<Item = &'a str>) -> Env {
    let mut env: Env = Vec::new();
    for line in records {
        if let Some(at) = line.find('=').filter(|&at| at > 0) {
            let (key, value) = (&line[..at], &line[at + 1..]);
            // A later line for the same name wins, as it does in an object.
            match env.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value.to_owned(),
                None => env.push((key.to_owned(), value.to_owned())),
            }
        }
    }
    env
}

/// The safe environment of `source`, with nothing passed through.
pub fn filter(source: impl Iterator<Item = (String, String)>) -> Env {
    safe_env(source)
}

/// The first entry of `path_env` holding `name` that can be run
/// (`findOnPath`).
pub fn find_on_path(name: &str, path_env: &str) -> Option<PathBuf> {
    find_launchable(name, Some(path_env), Platform::HOST).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_agents::launch::env::filter_env;

    fn pairs(list: &[(&str, &str)]) -> impl Iterator<Item = (String, String)> {
        list.iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn drops_credentials_and_agent_markers_whatever_their_case() {
        crate::adapters::testing::adapters();
        let kept = filter(pairs(&[
            ("PATH", "/bin"),
            ("HOME", "/h"),
            ("GITHUB_TOKEN", "x"),
            ("github_token_extra", "x"),
            ("SECRET_VORN_BOOTSTRAP_TOKEN", "x"),
            ("AcmeCode", "1"),
            ("ACME_CODE_SSE_PORT", "1"),
            ("ANTHROPIC_BASE_URL", "u"),
            ("ANTHROPIC_API_KEY", "k"),
        ]));
        let names: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["PATH", "HOME", "ANTHROPIC_BASE_URL"]);
    }

    #[test]
    fn passes_through_only_what_was_named_and_never_the_stripped() {
        crate::adapters::testing::adapters();
        let passthrough = vec!["ANTHROPIC_API_KEY".to_owned(), "ACMECODE".to_owned()];
        let kept = filter_env(
            pairs(&[
                ("anthropic_api_key", "k"),
                ("GITHUB_TOKEN", "x"),
                ("ACMECODE", "1"),
                ("PATH", "/bin"),
            ]),
            &passthrough,
        );
        let names: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["anthropic_api_key", "PATH"]);
    }

    #[test]
    fn reads_env_output_as_the_server_does() {
        let env = parse_env_output("A=1\nB=x=y\n=skip\nnoequals\nA=2\nEMPTY=\n");
        assert_eq!(
            env,
            [
                ("A".to_owned(), "2".to_owned()),
                ("B".to_owned(), "x=y".to_owned()),
                ("EMPTY".to_owned(), String::new())
            ]
        );
    }

    #[test]
    fn reads_only_what_is_between_the_sentinels() {
        let out = format!("motd=noise\n{SENTINEL}A=1\0MULTI=x\ny\0B=2\0{SENTINEL}bye=x");
        let env = parse_shell_output(&out).expect("env");
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["A", "MULTI", "B"]);
        assert_eq!(env[1].1, "x\ny");
        let plain = format!("{SENTINEL}A=1\nB=2\n{SENTINEL}");
        assert_eq!(parse_shell_output(&plain).map(|e| e.len()), Some(2));
        assert_eq!(parse_shell_output("A=1\n"), None, "no sentinel, no answer");
        assert_eq!(parse_shell_output(&format!("{SENTINEL}{SENTINEL}")), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_noisy_login_shell_still_gives_its_environment() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("noisy-sh");
        std::fs::write(
            &shell,
            "#!/bin/sh\necho 'Welcome=to the box'\nexec /bin/sh -c \"export MULTI='a\nb'; $2\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = shell_env(shell.to_str().unwrap()).expect("env");
        assert!(env.iter().all(|(k, _)| k != "Welcome"));
        assert!(env.iter().any(|(k, v)| k == "MULTI" && v == "a\nb"));
        let silent = dir.path().join("silent-sh");
        std::fs::write(&silent, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&silent, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(shell_env(silent.to_str().unwrap()).is_err());
    }

    #[test]
    fn appends_install_places_that_exist_and_are_missing() {
        let home = tempfile::tempdir().unwrap();
        let local = home.path().join(".local").join("bin");
        std::fs::create_dir_all(&local).unwrap();
        let gone = home.path().join("nowhere");
        let l = local.to_str().unwrap();
        let sep = if cfg!(windows) { ';' } else { ':' };
        let path = format!("/usr/bin{sep}/bin");
        let out = append_dirs(&path, &[local.clone(), gone, local.clone()], sep);
        assert_eq!(out, format!("/usr/bin{sep}/bin{sep}{l}"));
        assert_eq!(
            append_dirs(&out, std::slice::from_ref(&local), sep),
            out,
            "already there"
        );
        assert_eq!(append_dirs("", std::slice::from_ref(&local), sep), l);
    }

    #[cfg(unix)]
    #[test]
    fn a_minimal_path_gains_the_home_install_places() {
        let home = tempfile::tempdir().unwrap();
        let local = home.path().join(".local/bin");
        let nvm = home.path().join(".nvm/versions/node");
        for d in [
            &local,
            &nvm.join("v9.11.0/bin"),
            &nvm.join("v20.1.0/bin"),
            &nvm.join("junk"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        let h = home.path().to_str().unwrap();
        let env = with_fallback_path(vec![
            ("HOME".to_owned(), h.to_owned()),
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ]);
        let path = &env[1].1;
        assert!(
            path.starts_with("/usr/bin:/bin:"),
            "the shell's order comes first"
        );
        assert!(path.contains(&format!("{h}/.local/bin")));
        assert!(path.contains(&format!("{h}/.nvm/versions/node/v20.1.0/bin")));
        assert!(!path.contains("v9.11.0"), "only the newest node");
    }

    #[test]
    fn reads_the_path_reg_prints_and_expands_it() {
        let out = "\r\nHKEY_CURRENT_USER\\Environment\r\n    Path    REG_EXPAND_SZ    %USERPROFILE%\\.local\\bin;C:\\Tools\r\n\r\n";
        let path = parse_reg_path(out).expect("path");
        assert_eq!(path, "%USERPROFILE%\\.local\\bin;C:\\Tools");
        let lookup = |n: &str| (n == "USERPROFILE").then(|| "C:\\Users\\me".to_owned());
        assert_eq!(
            expand_percent(&path, lookup),
            "C:\\Users\\me\\.local\\bin;C:\\Tools"
        );
        assert_eq!(expand_percent("%NOPE%;50%", lookup), "%NOPE%;50%");
        assert_eq!(parse_reg_path("ERROR: not found"), None);
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_runnable_file_on_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("tool");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        let path = format!("/nonexistent: {} ", dir.path().display());
        assert_eq!(find_on_path("tool", &path), None);
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_on_path("tool", &path), Some(tool));
        assert_eq!(find_on_path("absent", &path), None);
    }
}

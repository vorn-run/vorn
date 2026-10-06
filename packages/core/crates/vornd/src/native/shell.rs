//! The `shell:*` calls that look at the machine rather than a session: the
//! programs on PATH, for the intent bar to complete a command with
//! (`listShellExecutables`), and the shells installed, with what each can
//! report as command blocks (`listInstalledShells`). Also what a local
//! shell session is launched with ([`Shells::setup`]), from the shim files
//! the server writes.
//!
//! Both are kept as the server keeps them: the programs for a minute, the
//! shells for as long as vornd runs. Finding the shells runs each one's
//! `--version`, which the server does on its event loop; here it holds only
//! a blocking thread.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use vorn_agents::launch::shell::{
    self as launch_shell, ShellContext, ShellFamily, ShellSetup, ShimError,
};
use vorn_agents::launch::Platform;

use super::env::SafeEnv;

/// How long the list of programs answers before PATH is read again.
const EXECUTABLES_TTL: Duration = Duration::from_secs(60);

/// What vornd keeps between calls.
#[derive(Debug, Default)]
pub struct Shells {
    executables: Mutex<Option<(Instant, Arc<Value>)>>,
    installed: OnceLock<Value>,
}

impl Shells {
    /// Every name on the safe environment's PATH that is not a directory,
    /// once each, in JavaScript's sort order.
    pub fn executables(&self, env: &Arc<SafeEnv>) -> Value {
        let mut kept = self.executables.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, names)) = kept.as_ref() {
            if at.elapsed() < EXECUTABLES_TTL {
                return names.as_ref().clone();
            }
        }
        let safe = env.get();
        let path = safe
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        let names = Arc::new(Value::Array(
            sorted_utf16(names_on_path(&path))
                .into_iter()
                .map(Value::String)
                .collect(),
        ));
        *kept = Some((Instant::now(), Arc::clone(&names)));
        names.as_ref().clone()
    }

    /// What a local shell session runs and is launched with
    /// (`getShellIntegration`): `shell`, else the default shell, with its
    /// integration's environment and arguments over the safe environment.
    /// An error means the server's shims are not a version this build
    /// knows, or not written yet, and the launch is the server's to make.
    pub fn setup(
        &self,
        env: &Arc<SafeEnv>,
        shell: Option<&str>,
        minimal_prompt: bool,
    ) -> Result<(String, ShellSetup), ShimError> {
        let var = |name: &str| std::env::var(name).ok();
        let shell = shell.map_or_else(
            || launch_shell::default_shell(None, Platform::HOST, var),
            str::to_owned,
        );
        let safe = env.get();
        let home = home_dir();
        let root = launch_shell::shim_root(Platform::HOST, var);
        let cx = ShellContext {
            minimal_prompt,
            env: &safe,
            home: &home,
            shim_root: &root,
        };
        let setup = launch_shell::shell_setup(&shell, &cx)?;
        Ok((shell, setup))
    }

    /// The shells on this machine, best first, found once.
    pub fn installed(&self) -> Value {
        self.installed
            .get_or_init(|| {
                let path = std::env::var("PATH").unwrap_or_default();
                Value::Array(installed_shells(&path))
            })
            .clone()
    }
}

fn names_on_path(path: &str) -> BTreeSet<String> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut names = BTreeSet::new();
    for dir in path.split(sep).filter(|d| !d.is_empty()) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // A link to a directory counts, as the server's directory
            // entries do not follow links either.
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            names.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names
}

/// `[...names].sort()`: by UTF-16 code units, which differs from Rust's
/// byte order only past the Basic Multilingual Plane.
fn sorted_utf16(names: BTreeSet<String>) -> Vec<String> {
    let mut list: Vec<String> = names.into_iter().collect();
    list.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    list
}

/// A shell family vornd knows the integration of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Zsh,
    Bash,
    Fish,
    PowerShell,
    Cmd,
}

impl Family {
    fn id(self) -> &'static str {
        match self {
            Family::Zsh => "zsh",
            Family::Bash => "bash",
            Family::Fish => "fish",
            Family::PowerShell => "powershell",
            Family::Cmd => "cmd",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Family::Zsh => "zsh",
            Family::Bash => "bash",
            Family::Fish => "fish",
            Family::PowerShell => "PowerShell",
            Family::Cmd => "Command Prompt",
        }
    }

    fn executables(self) -> &'static [&'static str] {
        match self {
            Family::Zsh => &["zsh"],
            Family::Bash => &["bash"],
            Family::Fish => &["fish"],
            // pwsh is PowerShell 7; powershell.exe is the 5.1 Windows ships.
            Family::PowerShell => &["pwsh", "powershell"],
            Family::Cmd => &["cmd"],
        }
    }

    /// What its blocks can show (`CAPABILITIES` through `describe`).
    fn blocks(self) -> Value {
        match self {
            Family::Zsh | Family::Bash | Family::Fish => {
                json!({ "level": "full", "limitation": null })
            }
            Family::PowerShell => json!({
                "level": "partial",
                "limitation": "Blocks appear once each command finishes"
            }),
            Family::Cmd => json!({
                "level": "limited",
                "limitation": "No exit status or command name on blocks"
            }),
        }
    }

    /// The family a shell's path names (`detectShellFamily`).
    fn of(path: &str) -> Option<Family> {
        ShellFamily::of(path).map(|family| match family {
            ShellFamily::Zsh => Family::Zsh,
            ShellFamily::Bash => Family::Bash,
            ShellFamily::Fish => Family::Fish,
            ShellFamily::PowerShell => Family::PowerShell,
            ShellFamily::Cmd => Family::Cmd,
        })
    }

    /// Places it can be without being on PATH.
    fn well_known(self) -> Vec<String> {
        if cfg!(windows) {
            let root = std::env::var("SystemRoot")
                .or_else(|_| std::env::var("windir"))
                .unwrap_or_else(|_| "C:\\Windows".to_owned());
            let at = |parts: &[&str]| {
                let mut p = std::path::PathBuf::from(&root);
                p.extend(parts);
                p.to_string_lossy().into_owned()
            };
            return match self {
                Family::PowerShell => vec![at(&[
                    "System32",
                    "WindowsPowerShell",
                    "v1.0",
                    "powershell.exe",
                ])],
                Family::Cmd => vec![at(&["System32", "cmd.exe"])],
                // Git for Windows ships bash outside PATH more often than not.
                Family::Bash => vec![
                    "C:\\Program Files\\Git\\bin\\bash.exe".to_owned(),
                    "C:\\Program Files (x86)\\Git\\bin\\bash.exe".to_owned(),
                ],
                _ => Vec::new(),
            };
        }
        match self {
            Family::Zsh => vec!["/bin/zsh".to_owned()],
            Family::Bash => vec!["/bin/bash".to_owned()],
            _ => Vec::new(),
        }
    }
}

/// Node's `os.homedir()`: `HOME`, or on Windows `USERPROFILE`.
fn home_dir() -> String {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(var).unwrap_or_default()
}

fn is_file(path: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

fn on_path(name: &str, path: &str) -> Vec<String> {
    let (sep, suffix) = if cfg!(windows) {
        (';', ".exe")
    } else {
        (':', "")
    };
    path.split(sep)
        .filter(|d| !d.is_empty())
        .map(|dir| {
            Path::new(dir)
                .join(format!("{name}{suffix}"))
                .to_string_lossy()
                .into_owned()
        })
        .filter(|candidate| is_file(candidate))
        .collect()
}

/// The shells found on `path` or where they are known to live, best first
/// (`listInstalledShells`).
fn installed_shells(path: &str) -> Vec<Value> {
    let order: &[Family] = if cfg!(windows) {
        &[Family::PowerShell, Family::Bash, Family::Cmd]
    } else {
        &[Family::Zsh, Family::Fish, Family::Bash, Family::PowerShell]
    };
    let mut seen = std::collections::HashSet::new();
    let mut shells = Vec::new();
    for &family in order {
        let candidates = family
            .executables()
            .iter()
            .flat_map(|name| on_path(name, path))
            .chain(family.well_known().into_iter().filter(|p| is_file(p)));
        for candidate in candidates {
            if !seen.insert(candidate.to_lowercase()) {
                continue;
            }
            // A name that resolved to something else entirely, such as a
            // wrapper called "bash" that is not bash, is not listed.
            if Family::of(&candidate) != Some(family) {
                continue;
            }
            let version = read_version(family, &candidate);
            let name = match (&version, family) {
                (Some(v), Family::PowerShell) => {
                    format!("PowerShell {}", v.chars().next().unwrap_or_default())
                }
                _ => family.label().to_owned(),
            };
            shells.push(json!({
                "family": family.id(),
                "name": name,
                "path": candidate,
                "version": version,
                "blocks": family.blocks(),
            }));
        }
    }
    shells
}

/// The first `N.N` or `N.N.N` in what `--version` prints; `None` when the
/// shell fails, takes too long or prints none.
fn read_version(family: Family, shell: &str) -> Option<String> {
    if family == Family::Cmd {
        return None;
    }
    // A cold PowerShell loads .NET first and can take seconds.
    let limit = if family == Family::PowerShell {
        Duration::from_secs(6)
    } else {
        Duration::from_secs(2)
    };
    let mut child = Command::new(shell)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
        out
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = reader.join().ok()?;
    if !status.success() {
        return None;
    }
    first_version(&String::from_utf8_lossy(&out))
}

/// `/(\d+\.\d+(?:\.\d+)?)/`.
fn first_version(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let digits_from = |at: usize| b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut i = 0;
    while i < b.len() {
        let major = digits_from(i);
        if major == 0 {
            i += 1;
            continue;
        }
        let dot = i + major;
        if b.get(dot) == Some(&b'.') {
            let minor = digits_from(dot + 1);
            if minor > 0 {
                let mut end = dot + 1 + minor;
                if b.get(end) == Some(&b'.') {
                    let patch = digits_from(end + 1);
                    if patch > 0 {
                        end += 1 + patch;
                    }
                }
                return Some(text[i..end].to_owned());
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_first_version_number() {
        assert_eq!(
            first_version("zsh 5.9 (arm64-apple-darwin)").as_deref(),
            Some("5.9")
        );
        assert_eq!(
            first_version("GNU bash, version 3.2.57(1)-release").as_deref(),
            Some("3.2.57")
        );
        assert_eq!(first_version("PowerShell 7.6.4").as_deref(), Some("7.6.4"));
        assert_eq!(first_version("v12.x 1.2").as_deref(), Some("1.2"));
        assert_eq!(first_version("no version"), None);
    }

    #[test]
    fn knows_a_shell_by_its_file_name() {
        assert_eq!(Family::of("/usr/bin/zsh"), Some(Family::Zsh));
        assert_eq!(
            Family::of("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\PowerShell.EXE"),
            Some(Family::PowerShell)
        );
        assert_eq!(Family::of("/opt/bin/pwsh"), Some(Family::PowerShell));
        assert_eq!(Family::of("/bin/sh"), None);
    }

    #[test]
    fn sets_up_a_shell_that_needs_no_shims() {
        let shells = Shells::default();
        let env = SafeEnv::new();
        let (shell, setup) = shells.setup(&env, Some("/bin/sh"), true).unwrap();
        assert_eq!((shell.as_str(), setup), ("/bin/sh", ShellSetup::default()));
        let (_, setup) = shells.setup(&env, Some("cmd.exe"), true).unwrap();
        assert_eq!(setup.env[0].0, "PROMPT");
        assert_eq!(setup.args, None);
    }

    #[test]
    fn sorts_as_javascript_sorts() {
        let names: BTreeSet<String> = ["b", "B", "\u{FF5E}", "😀", "a"].map(String::from).into();
        assert_eq!(sorted_utf16(names), ["B", "a", "b", "😀", "\u{FF5E}"]);
    }

    #[cfg(unix)]
    #[test]
    fn lists_files_on_path_but_not_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tool"), "").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::os::unix::fs::symlink("/nonexistent", dir.path().join("dangling")).unwrap();
        let path = format!("{}::/nonexistent", dir.path().display());
        let names: Vec<String> = names_on_path(&path).into_iter().collect();
        assert_eq!(names, ["dangling", "tool"]);
    }
}

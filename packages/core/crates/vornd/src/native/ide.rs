//! The editors and file managers a project can be opened in, as the server's
//! `ide-detector` finds and starts them.

use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::env::SafeEnv;

/// One editor the server knows how to look for.
#[derive(Debug, Clone, Copy)]
struct Definition {
    id: &'static str,
    name: &'static str,
    /// Found by this app bundle existing; `None` is found on PATH.
    app_path: Option<&'static str>,
    command: &'static str,
}

const fn def(
    id: &'static str,
    name: &'static str,
    app_path: Option<&'static str>,
    command: &'static str,
) -> Definition {
    Definition {
        id,
        name,
        app_path,
        command,
    }
}

const MAC: &[Definition] = &[
    def(
        "vscode",
        "VS Code",
        Some("/Applications/Visual Studio Code.app"),
        "code",
    ),
    def(
        "vscode-insiders",
        "VS Code Insiders",
        Some("/Applications/Visual Studio Code - Insiders.app"),
        "code-insiders",
    ),
    def(
        "cursor",
        "Cursor",
        Some("/Applications/Cursor.app"),
        "cursor",
    ),
    def(
        "windsurf",
        "Windsurf",
        Some("/Applications/Windsurf.app"),
        "windsurf",
    ),
    def("zed", "Zed", Some("/Applications/Zed.app"), "zed"),
    def(
        "sublime",
        "Sublime Text",
        Some("/Applications/Sublime Text.app"),
        "subl",
    ),
    def(
        "webstorm",
        "WebStorm",
        Some("/Applications/WebStorm.app"),
        "webstorm",
    ),
    def(
        "intellij",
        "IntelliJ IDEA",
        Some("/Applications/IntelliJ IDEA.app"),
        "idea",
    ),
    def("xcode", "Xcode", Some("/Applications/Xcode.app"), "xed"),
    def("terminal", "Terminal", None, "open -a Terminal"),
    def("finder", "Finder", None, "open"),
];

const WINDOWS: &[Definition] = &[
    def("vscode", "VS Code", None, "code"),
    def("vscode-insiders", "VS Code Insiders", None, "code-insiders"),
    def("cursor", "Cursor", None, "cursor"),
    def("windsurf", "Windsurf", None, "windsurf"),
    def("sublime", "Sublime Text", None, "subl"),
    def("webstorm", "WebStorm", None, "webstorm"),
    def("intellij", "IntelliJ IDEA", None, "idea"),
    def("explorer", "Explorer", None, "explorer"),
];

const LINUX: &[Definition] = &[
    def("vscode", "VS Code", None, "code"),
    def("vscode-insiders", "VS Code Insiders", None, "code-insiders"),
    def("cursor", "Cursor", None, "cursor"),
    def("windsurf", "Windsurf", None, "windsurf"),
    def("zed", "Zed", None, "zed"),
    def("sublime", "Sublime Text", None, "subl"),
    def("webstorm", "WebStorm", None, "webstorm"),
    def("intellij", "IntelliJ IDEA", None, "idea"),
    def("file-manager", "File Manager", None, "xdg-open"),
];

/// The server's list for this platform; anything not Windows or Linux is a Mac.
fn definitions() -> &'static [Definition] {
    if cfg!(windows) {
        WINDOWS
    } else if cfg!(target_os = "linux") {
        LINUX
    } else {
        MAC
    }
}

/// An editor found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub id: &'static str,
    pub name: &'static str,
    pub command: &'static str,
}

/// The editors found, looked for once: the server keeps its first answer for
/// as long as it runs, and so does vornd.
#[derive(Debug, Default)]
pub struct Ides {
    found: OnceLock<Vec<Detected>>,
}

impl Ides {
    pub fn detect(&self, env: &Arc<SafeEnv>) -> &[Detected] {
        self.found.get_or_init(|| {
            definitions()
                .iter()
                .filter(|d| match d.app_path {
                    Some(app) => std::path::Path::new(app).exists(),
                    None => command_exists(d.command.split(' ').next().unwrap_or(""), env),
                })
                .map(|d| Detected {
                    id: d.id,
                    name: d.name,
                    command: d.command,
                })
                .collect()
        })
    }

    pub fn detect_json(&self, env: &Arc<SafeEnv>) -> Value {
        Value::Array(
            self.detect(env)
                .iter()
                .map(|d| json!({ "id": d.id, "name": d.name, "command": d.command }))
                .collect(),
        )
    }

    /// Starts the editor `id` on `project`, detached, and does not wait for
    /// it. An editor not found is nothing to do, as for the server.
    pub fn open(&self, id: &str, project: &str, env: &Arc<SafeEnv>) {
        let Some(ide) = self.detect(env).iter().find(|i| i.id == id) else {
            return;
        };
        let mut parts = ide.command.split(' ');
        let program = parts.next().unwrap_or(ide.command);
        let mut cmd = if cfg!(windows) {
            // The server spawns with `shell: true` there, so `code` finds `code.cmd`.
            let line = std::iter::once(program)
                .chain(parts)
                .chain(std::iter::once(project))
                .collect::<Vec<_>>()
                .join(" ");
            let mut cmd = Command::new("cmd.exe");
            cmd.args(["/d", "/s", "/c", &line]);
            cmd
        } else {
            let mut cmd = Command::new(program);
            cmd.args(parts).arg(project);
            cmd
        };
        cmd.env_clear()
            .envs(env.get())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach(&mut cmd);
        if let Ok(mut child) = cmd.spawn() {
            // Reaped on a thread of its own so it never lingers as a zombie.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

/// Its own process group, so it outlives vornd and is not signalled with it.
#[cfg(unix)]
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
}

/// Whether `which` (`where` on Windows) finds `cmd` within three seconds.
fn command_exists(cmd: &str, env: &Arc<SafeEnv>) -> bool {
    let finder = if cfg!(windows) { "where" } else { "which" };
    let Ok(mut child) = Command::new(finder)
        .arg(cmd)
        .env_clear()
        .envs(env.get())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_lists_unique_ids_and_a_file_manager() {
        for list in [MAC, WINDOWS, LINUX] {
            let mut ids: Vec<&str> = list.iter().map(|d| d.id).collect();
            let n = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), n);
            assert!(list.iter().any(|d| d.app_path.is_none()));
        }
    }

    #[test]
    fn looks_once_and_keeps_the_answer() {
        let ides = Ides::default();
        let env = SafeEnv::new();
        let first = ides.detect(&env).to_vec();
        assert_eq!(ides.detect(&env), first.as_slice());
        // An editor that was not found is nothing to open.
        ides.open("no-such-editor", "/tmp", &env);
    }
}

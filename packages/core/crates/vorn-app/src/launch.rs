//! Starting vornd when none is running, as the Electron app does: the vornd
//! shipped with this app, detached, serving the data directory, logging to
//! its `server.log`, and stopping once nothing has used it for a while.
//!
//! vornd is found where the packages put it:
//!
//! - beside `vorn-app` (the Windows and Linux archives, a cargo target dir),
//! - in `Contents/Resources/vornd` of the macOS bundle,
//!
//! or at `VORN_VORND_PATH`. vorn-sessiond is the one beside vornd.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Names a vornd elsewhere, as the CLI and the Electron app honour it.
pub const VORND_PATH_ENV: &str = "VORN_VORND_PATH";
/// Lets a debug build start vornd on the default data directory.
pub const ALLOW_DEFAULT_ENV: &str = "VORN_ALLOW_DEFAULT_DATA_DIR";
/// Where a started vornd's output goes, in the data directory.
pub const LOG_FILE: &str = "server.log";

/// A vornd to start, and the session holder beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub vornd: PathBuf,
    pub sessiond: Option<PathBuf>,
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

impl Server {
    fn at(vornd: PathBuf) -> Server {
        let sessiond = vornd.with_file_name(exe("vorn-sessiond"));
        Server {
            sessiond: sessiond.is_file().then_some(sessiond),
            vornd,
        }
    }

    /// The vornd this app ships with, if it has one.
    pub fn locate() -> Option<Server> {
        if let Some(named) = std::env::var_os(VORND_PATH_ENV).filter(|v| !v.is_empty()) {
            let named = PathBuf::from(named);
            return named.is_file().then(|| Server::at(named));
        }
        let here = std::env::current_exe().ok()?;
        let here = here.canonicalize().unwrap_or(here);
        Server::beside(here.parent()?)
    }

    /// The vornd for an app whose executable is in `dir`.
    pub fn beside(dir: &Path) -> Option<Server> {
        [
            dir.join(exe("vornd")),
            dir.join("..").join("Resources").join("vornd").join("vornd"),
        ]
        .into_iter()
        .find(|p| p.is_file())
        .map(Server::at)
    }

    /// The command that serves `data_dir`, with the Electron app's arguments.
    pub fn command(&self, data_dir: &Path) -> Command {
        let mut command = vorn_spawn::command(&self.vornd);
        command.arg("--data-dir").arg(data_dir);
        if let Some(sessiond) = &self.sessiond {
            command.arg("--sessiond").arg(sessiond);
        }
        // An app's server stops once nothing has used it for a while.
        command.arg("--idle-exit");
        command
            .env("VORN_BUILD_CHANNEL", "packaged")
            .env("VORN_APP_VERSION", env!("CARGO_PKG_VERSION"))
            .current_dir(data_dir);
        command
    }

    /// Starts vornd detached, so it outlives this app. A rival started at
    /// the same moment stands down by itself.
    pub fn start(&self, data_dir: &Path) -> Result<(), String> {
        refuse_default(data_dir)?;
        create_private_dir(data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
        let log = data_dir.join(LOG_FILE);
        let out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| format!("{}: {e}", log.display()))?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let mut command = self.command(data_dir);
        command.stdin(Stdio::null()).stdout(out).stderr(err);
        detach(&mut command);
        // Not waited for: it keeps running after this app quits.
        command
            .spawn()
            .map(drop)
            .map_err(|e| format!("{}: {e}", self.vornd.display()))
    }
}

/// A debug build never starts vornd on `~/.vorn` unless told to, so a test
/// that forgets its own directory cannot reach a person's data.
fn refuse_default(dir: &Path) -> Result<(), String> {
    if !cfg!(debug_assertions) || std::env::var(ALLOW_DEFAULT_ENV).is_ok_and(|v| v == "1") {
        return Ok(());
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let Some(home) = home.map(PathBuf::from) else {
        return Ok(());
    };
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    if canon(dir) == canon(&home.join(".vorn")) {
        return Err(format!(
            "a debug build will not start vornd on {}; set {ALLOW_DEFAULT_ENV}=1 to allow it",
            dir.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Its own process group, so a signal meant for the app misses it.
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use vorn_spawn::{Hidden, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
    command.hidden_with(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vorn-app-launch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn finds_vornd_beside_the_app_or_in_the_bundle() {
        let d = dir("find");
        let flat = d.join("flat");
        std::fs::create_dir_all(&flat).unwrap();
        assert_eq!(Server::beside(&flat), None);
        std::fs::write(flat.join(exe("vornd")), "").unwrap();
        std::fs::write(flat.join(exe("vorn-sessiond")), "").unwrap();
        let got = Server::beside(&flat).unwrap();
        assert_eq!(got.vornd, flat.join(exe("vornd")));
        assert_eq!(got.sessiond, Some(flat.join(exe("vorn-sessiond"))));

        let macos = d.join("Vorn Native.app/Contents/MacOS");
        let res = d.join("Vorn Native.app/Contents/Resources/vornd");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(&res).unwrap();
        std::fs::write(res.join("vornd"), "").unwrap();
        let got = Server::beside(&macos).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert!(got.vornd.ends_with("Resources/vornd/vornd"), "{got:?}");
        assert_eq!(got.sessiond, None);
    }

    #[test]
    fn serves_the_data_directory_as_the_electron_app_does() {
        let server = Server {
            vornd: PathBuf::from("/b/vornd"),
            sessiond: Some(PathBuf::from("/b/vorn-sessiond")),
        };
        let command = server.command(Path::new("/data"));
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--data-dir",
                "/data",
                "--sessiond",
                "/b/vorn-sessiond",
                "--idle-exit"
            ]
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/data")));
    }
}

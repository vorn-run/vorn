//! Starting vornd when none is running: the vornd shipped with this app,
//! detached, serving the data directory and the web client, logging to its
//! `server.log`, and stopping once nothing has used it for a while.
//!
//! vornd is found where the packages put it:
//!
//! - in `resources/vornd` beside the executable (Windows and Linux),
//! - in `Contents/Resources/vornd` of the macOS bundle,
//! - beside the executable (a cargo target dir),
//!
//! or at `VORN_VORND_PATH`. vorn-sessiond is the one beside vornd, and the
//! web client is `web/dist` beside that `vornd` directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use web_time::{Duration, Instant};

use serde_json::Value;

use crate::client::rpc::{Event, Rpc};
use crate::client::Endpoint;

/// Names a vornd elsewhere, as the CLI honours it.
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
    /// The web client's build, served under `/app`.
    pub web: Option<PathBuf>,
    /// Shipped in an installed app rather than found in a build tree.
    pub packaged: bool,
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// The installed app's resources for an executable in `dir`: `resources`
/// beside it, or the macOS bundle's `Contents/Resources`.
pub fn resources(dir: &Path) -> Option<PathBuf> {
    [dir.join("resources"), dir.join("..").join("Resources")]
        .into_iter()
        .find(|p| p.join("vornd").is_dir())
}

impl Server {
    fn at(vornd: PathBuf, packaged: bool) -> Server {
        let sessiond = vornd.with_file_name(exe("vorn-sessiond"));
        let web = vornd
            .parent()
            .and_then(Path::parent)
            .map(|r| r.join("web").join("dist"))
            .filter(|w| w.join("index.html").is_file());
        Server {
            sessiond: sessiond.is_file().then_some(sessiond),
            web,
            vornd,
            packaged,
        }
    }

    /// The vornd this app ships with, if it has one.
    pub fn locate() -> Option<Server> {
        if let Some(named) = std::env::var_os(VORND_PATH_ENV).filter(|v| !v.is_empty()) {
            let named = PathBuf::from(named);
            return named.is_file().then(|| Server::at(named, false));
        }
        let here = std::env::current_exe().ok()?;
        let here = here.canonicalize().unwrap_or(here);
        Server::beside(here.parent()?)
    }

    /// The vornd for an app whose executable is in `dir`.
    pub fn beside(dir: &Path) -> Option<Server> {
        if let Some(vornd) = resources(dir)
            .map(|r| r.join("vornd").join(exe("vornd")))
            .filter(|p| p.is_file())
        {
            return Some(Server::at(vornd, true));
        }
        let flat = dir.join(exe("vornd"));
        flat.is_file().then(|| Server::at(flat, false))
    }

    /// What vornd reports as its kind of build, as the server judges adoption by it.
    pub fn channel(&self) -> &'static str {
        if self.packaged {
            "packaged"
        } else {
            "dev"
        }
    }

    /// The command that serves `data_dir`.
    pub fn command(&self, data_dir: &Path) -> Command {
        let mut command = vorn_spawn::command(&self.vornd);
        command.arg("--data-dir").arg(data_dir);
        if let Some(sessiond) = &self.sessiond {
            command.arg("--sessiond").arg(sessiond);
        }
        if let Some(web) = &self.web {
            command.arg("--web").arg(web);
        }
        // An app's server stops once nothing has used it for a while.
        command.arg("--idle-exit");
        command
            .env("VORN_BUILD_CHANNEL", self.channel())
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

/// What a running vornd says it is (`server:identity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub app_version: String,
    pub build_channel: String,
}

impl Identity {
    fn from(params: &Value) -> Identity {
        let field = |k: &str| params[k].as_str().unwrap_or("unknown").to_owned();
        Identity {
            app_version: field("appVersion"),
            build_channel: field("buildChannel"),
        }
    }
}

/// `replacesIncumbent`: whether a running vornd is an older build this app
/// should replace with its own. Its terminals live in the session holder,
/// which the new vornd takes over. Only between packaged builds that both say
/// which they are: a dev build has one version across every edit.
pub fn replaces(incumbent: &Identity, version: &str, channel: &str) -> bool {
    incumbent.build_channel == "packaged"
        && channel == "packaged"
        && incumbent.app_version != "unknown"
        && incumbent.app_version != version
}

/// After an update: stops the running vornd for `dir` when it is an older
/// packaged build, so the caller starts the one shipped with this app.
/// Whether it stopped one.
pub fn retire_older(dir: &Path, server: &Server) -> bool {
    let Ok(ep) = Endpoint::find(dir) else {
        return false;
    };
    let Ok((rpc, events)) = Rpc::connect(&ep, std::sync::Arc::new(|| {})) else {
        return false;
    };
    let until = Instant::now() + Duration::from_secs(2);
    let identity = loop {
        let left = until.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(Event::Notification { method, params }) if method == "server:identity" => {
                break Identity::from(&params);
            }
            Ok(_) => {}
            Err(_) => return false,
        }
    };
    if !replaces(&identity, env!("CARGO_PKG_VERSION"), server.channel()) {
        return false;
    }
    eprintln!(
        "vorn-app: the running vornd is {}; replacing it with this build",
        identity.app_version
    );
    let _ = rpc.call("server:shutdown", Value::Null);
    drop(rpc);
    // Gone once its port refuses a connection.
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], ep.port));
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
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

        assert!(!got.packaged);
        assert_eq!(got.web, None);

        let macos = d.join("Vorn.app/Contents/MacOS");
        let res = d.join("Vorn.app/Contents/Resources/vornd");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(&res).unwrap();
        std::fs::write(res.join("vornd"), "").unwrap();
        let got = Server::beside(&macos).unwrap();
        assert!(got.vornd.ends_with("Resources/vornd/vornd"), "{got:?}");
        assert_eq!(got.sessiond, None);
        assert!(got.packaged);

        let installed = d.join("installed");
        let res = installed.join("resources");
        std::fs::create_dir_all(res.join("vornd")).unwrap();
        std::fs::create_dir_all(res.join("web/dist")).unwrap();
        std::fs::write(res.join("vornd").join(exe("vornd")), "").unwrap();
        std::fs::write(res.join("web/dist/index.html"), "").unwrap();
        let got = Server::beside(&installed).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(got.vornd, res.join("vornd").join(exe("vornd")));
        assert_eq!(got.web, Some(res.join("web").join("dist")));
        assert_eq!(got.channel(), "packaged");
    }

    #[test]
    fn replaces_only_an_older_packaged_build() {
        let id = |v: &str, c: &str| Identity {
            app_version: v.into(),
            build_channel: c.into(),
        };
        assert!(replaces(
            &id("0.8.0-beta.8", "packaged"),
            "0.8.0-beta.9",
            "packaged"
        ));
        assert!(!replaces(
            &id("0.8.0-beta.9", "packaged"),
            "0.8.0-beta.9",
            "packaged"
        ));
        assert!(!replaces(
            &id("unknown", "packaged"),
            "0.8.0-beta.9",
            "packaged"
        ));
        assert!(!replaces(
            &id("0.8.0-beta.8", "dev"),
            "0.8.0-beta.9",
            "packaged"
        ));
        assert!(!replaces(
            &id("0.8.0-beta.8", "packaged"),
            "0.8.0-beta.9",
            "dev"
        ));
        assert_eq!(
            Identity::from(&serde_json::json!({ "appVersion": "1.0.0" })),
            id("1.0.0", "unknown")
        );
    }

    #[test]
    fn serves_the_data_directory_and_the_web_client() {
        let server = Server {
            vornd: PathBuf::from("/b/vornd"),
            sessiond: Some(PathBuf::from("/b/vorn-sessiond")),
            web: Some(PathBuf::from("/b/web/dist")),
            packaged: true,
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
                "--web",
                "/b/web/dist",
                "--idle-exit"
            ]
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/data")));
    }
}

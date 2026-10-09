//! Running the Vorn server, vornd: what `vorn server serve` does, and what a
//! client command does when no server is running.
//!
//! vornd is found beside this binary:
//!
//! - In the packaged app both are in `resources/vornd`, and the web client
//!   it serves is `resources/web/dist`.
//! - In a checkout it is `packages/core/vornd` beside `packages/core/vorn`, or
//!   in the same `target/<profile>` directory; the web client is
//!   `packages/web/dist`.
//!
//! `VORN_VORND_PATH` names a vornd elsewhere.

use std::path::{Path, PathBuf};
use std::process::Command;

/// How to start the server.
#[derive(Debug, Clone)]
pub struct ServerCommand {
    vornd: PathBuf,
    sessiond: Option<PathBuf>,
    web: Option<PathBuf>,
}

impl ServerCommand {
    /// vornd serving `data_dir`, on `port` and `host` when given.
    pub fn command(&self, data_dir: &Path, port: Option<u16>, host: Option<&str>) -> Command {
        // spawn-visible: `vorn server serve` runs it in the caller's console; a detached start hides it.
        let mut command = Command::new(&self.vornd);
        command.arg("--data-dir").arg(data_dir);
        if let Some(sessiond) = &self.sessiond {
            command.arg("--sessiond").arg(sessiond);
        }
        if let Some(web) = &self.web {
            command.arg("--web").arg(web);
        }
        if let Some(port) = port {
            command.arg("--port").arg(port.to_string());
        }
        if let Some(host) = host {
            command.arg("--host").arg(host);
        }
        command
    }
}

/// No vornd anywhere this binary looked.
#[derive(Debug)]
pub struct NotFound {
    pub looked: Vec<PathBuf>,
}

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "This vorn cannot start a Vorn server: vornd was not found beside it."
        )?;
        writeln!(f, "Looked for:")?;
        for path in &self.looked {
            writeln!(f, "  {}", path.display())?;
        }
        write!(
            f,
            "Start the Vorn app, or run `vorn server serve` from the Vorn install, and try again.\nSet VORN_VORND_PATH to a vornd to start one from elsewhere."
        )
    }
}

impl std::error::Error for NotFound {}

/// Lets a debug build use the default data directory.
pub const ALLOW_DEFAULT_VAR: &str = "VORN_ALLOW_DEFAULT_DATA_DIR";

/// Refuses `dir` when it is the default data directory, `~/.vorn`, and this is
/// a debug build not told otherwise: a test that forgets its own directory
/// must never reach a person's data.
pub fn refuse_default(dir: &Path) -> Result<(), String> {
    let allowed = std::env::var(ALLOW_DEFAULT_VAR).is_ok_and(|v| v == "1");
    refuse_default_in(dir, &crate::rpc::home_dir(), allowed)
}

fn refuse_default_in(dir: &Path, home: &Path, allowed: bool) -> Result<(), String> {
    if !cfg!(debug_assertions) || allowed {
        return Ok(());
    }
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    if canon(dir) == canon(&home.join(".vorn")) {
        return Err(format!(
            "a debug build will not use the default data directory {}; set {ALLOW_DEFAULT_VAR}=1 to allow it",
            dir.display()
        ));
    }
    Ok(())
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// The web client's build, looked for above `dir`.
fn web_client(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().take(5).find_map(|up| {
        [
            up.join("web").join("dist"),
            up.join("packages").join("web").join("dist"),
        ]
        .into_iter()
        .find(|d| d.is_dir())
    })
}

fn found(vornd: PathBuf) -> ServerCommand {
    let dir = vornd.parent().map(Path::to_owned).unwrap_or_default();
    let sessiond = dir.join(exe("vorn-sessiond"));
    ServerCommand {
        web: web_client(&dir),
        sessiond: sessiond.is_file().then_some(sessiond),
        vornd,
    }
}

/// Finds vornd.
pub fn locate() -> Result<ServerCommand, NotFound> {
    let mut looked = Vec::new();
    if let Some(named) = std::env::var_os("VORN_VORND_PATH").filter(|v| !v.is_empty()) {
        let named = PathBuf::from(named);
        if named.is_file() {
            return Ok(found(named));
        }
        looked.push(named);
        return Err(NotFound { looked });
    }
    let here = std::env::current_exe().and_then(|p| p.canonicalize().or(Ok(p)));
    let Some(dir) = here.ok().and_then(|p| p.parent().map(Path::to_owned)) else {
        return Err(NotFound { looked });
    };
    let candidate = dir.join(exe("vornd"));
    if candidate.is_file() {
        return Ok(found(candidate));
    }
    looked.push(candidate);
    Err(NotFound { looked })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_debug_build_keeps_off_the_default_data_directory() {
        let home = tempfile::tempdir().unwrap();
        let default = home.path().join(".vorn");
        assert_eq!(
            refuse_default_in(&default, home.path(), false).is_err(),
            cfg!(debug_assertions)
        );
        assert!(refuse_default_in(&default, home.path(), true).is_ok());
        assert!(refuse_default_in(&home.path().join("x"), home.path(), false).is_ok());
    }

    #[test]
    fn runs_vornd_as_the_server_for_a_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("resources").join("vornd");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(dir.path().join("resources").join("web").join("dist")).unwrap();
        std::fs::write(bin.join(exe("vornd")), "").unwrap();
        std::fs::write(bin.join(exe("vorn-sessiond")), "").unwrap();
        let server = found(bin.join(exe("vornd")));
        let command = server.command(Path::new("/data"), Some(5000), Some("0.0.0.0"));
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let web = dir.path().join("resources").join("web").join("dist");
        assert_eq!(
            args,
            [
                "--data-dir".to_owned(),
                "/data".into(),
                "--sessiond".into(),
                bin.join(exe("vorn-sessiond"))
                    .to_string_lossy()
                    .into_owned(),
                "--web".into(),
                web.to_string_lossy().into_owned(),
                "--port".into(),
                "5000".into(),
                "--host".into(),
                "0.0.0.0".into(),
            ]
        );
        // Without a session holder or a web client, only the directory.
        let bare = tempfile::tempdir().unwrap();
        let alone = found(bare.path().join(exe("vornd")));
        assert_eq!(
            alone
                .command(Path::new("/d"), None, None)
                .get_args()
                .count(),
            2
        );
    }
}

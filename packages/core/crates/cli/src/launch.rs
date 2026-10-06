//! Running the Node server: what `vorn server serve` does, and what a client
//! command does when no server is running.
//!
//! The server is still the TypeScript one, so this binary finds its command
//! line entry, `cli.cjs`, and the runtime to run it with, from where this
//! binary sits:
//!
//! - In the packaged app it is `resources/vornd/vorn`, beside
//!   `resources/server/cli.cjs`, and the app's own executable is the runtime
//!   (`ELECTRON_RUN_AS_NODE`), set up as the `vorn` shell command sets it up.
//! - In a checkout it is `packages/core/vorn` (or `target/<profile>/vorn`),
//!   and the entry is `packages/server/dist/cli.cjs`, run with `node`; without
//!   a build, `packages/server/src/cli.ts` through `npx tsx`, as the
//!   TypeScript command re-invokes itself from source.
//!
//! `VORN_SERVER_ENTRY` names an entry elsewhere, run with `node` (or `npx tsx`
//! for a `.ts` file).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How to start the server's command line.
#[derive(Debug, Clone)]
pub struct ServerCommand {
    program: PathBuf,
    /// Before the server's own arguments: the entry, and a loader for source.
    leading: Vec<OsString>,
    env: Vec<(&'static str, OsString)>,
}

impl ServerCommand {
    /// A command running the server's CLI with `args`.
    pub fn command(&self, args: &[String]) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.leading).args(args);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

/// No server entry anywhere this binary looked.
#[derive(Debug)]
pub struct NotFound {
    pub looked: Vec<PathBuf>,
}

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "This vorn cannot start a Vorn server: the server it runs was not found beside it."
        )?;
        writeln!(f, "Looked for:")?;
        for path in &self.looked {
            writeln!(f, "  {}", path.display())?;
        }
        write!(
            f,
            "Start the Vorn app, or run `vorn server serve` from the Vorn install, and try again.\nSet VORN_SERVER_ENTRY to the server's cli.cjs to start one from elsewhere."
        )
    }
}

impl std::error::Error for NotFound {}

/// An executable on `PATH`.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            name.to_owned(),
        ]
    } else {
        vec![name.to_owned()]
    };
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|candidate| candidate.is_file())
}

/// `node <entry>`, or `npx tsx <entry>` for TypeScript source.
fn with_node(entry: &Path) -> Option<ServerCommand> {
    if entry.extension().is_some_and(|e| e == "ts") {
        return Some(ServerCommand {
            program: on_path("npx")?,
            leading: vec!["tsx".into(), entry.into()],
            env: Vec::new(),
        });
    }
    Some(ServerCommand {
        program: on_path("node")?,
        leading: vec![entry.into()],
        env: Vec::new(),
    })
}

/// The app's executable, given its resources directory.
fn app_executable(resources: &Path) -> Option<PathBuf> {
    let parent = resources.parent()?;
    let candidate = if cfg!(target_os = "macos") {
        parent.join("MacOS").join("Vorn")
    } else if cfg!(windows) {
        parent.join("Vorn.exe")
    } else {
        parent.join("vorn")
    };
    candidate.is_file().then_some(candidate)
}

/// The packaged app's server, run by the app as Node, as the shell command runs it.
fn packaged(resources: &Path, entry: &Path) -> Option<ServerCommand> {
    let Some(app) = app_executable(resources) else {
        return with_node(entry);
    };
    let unpacked = resources.join("app.asar.unpacked").join("node_modules");
    let mut node_path = OsString::from(resources.join("app.asar").join("node_modules"));
    node_path.push(if cfg!(windows) { ";" } else { ":" });
    node_path.push(&unpacked);
    Some(ServerCommand {
        program: app,
        leading: vec![entry.into()],
        env: vec![
            ("ELECTRON_RUN_AS_NODE", "1".into()),
            ("VORN_NATIVE_MODULES_PATH", unpacked.into()),
            ("NODE_PATH", node_path),
        ],
    })
}

/// Finds the server's command line entry and how to run it.
pub fn locate() -> Result<ServerCommand, NotFound> {
    let mut looked = Vec::new();

    if let Some(entry) = std::env::var_os("VORN_SERVER_ENTRY").filter(|v| !v.is_empty()) {
        let entry = PathBuf::from(entry);
        if entry.is_file() {
            if let Some(found) = with_node(&entry) {
                return Ok(found);
            }
        }
        looked.push(entry);
        return Err(NotFound { looked });
    }

    let exe = std::env::current_exe().and_then(|p| p.canonicalize().or(Ok(p)));
    let Some(dir) = exe.ok().and_then(|p| p.parent().map(Path::to_owned)) else {
        return Err(NotFound { looked });
    };

    // Packaged: resources/vornd/vorn beside resources/server/cli.cjs.
    if let Some(resources) = dir.parent() {
        let entry = resources.join("server").join("cli.cjs");
        if entry.is_file() {
            if let Some(found) = packaged(resources, &entry) {
                return Ok(found);
            }
        }
        looked.push(entry);
    }

    // A checkout: packages/core/vorn, or packages/core/target/<profile>/vorn.
    for packages in [dir.parent(), dir.ancestors().nth(3)].into_iter().flatten() {
        let server = packages.join("server");
        for entry in [
            server.join("dist").join("cli.cjs"),
            server.join("src").join("cli.ts"),
        ] {
            if entry.is_file() {
                if let Some(found) = with_node(&entry) {
                    return Ok(found);
                }
            }
            looked.push(entry);
        }
    }
    Err(NotFound { looked })
}

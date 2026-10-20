//! Installing an extension from git, linking a folder, removing one, and
//! packing one for the catalog.
//!
//! An install runs nothing of the extension's before the person confirms:
//! [`Installs::inspect_git`] clones with hooks off and reads the manifest
//! into a [`Sheet`] that names the publisher, every command, every host, the
//! Vorn methods it may call and what it adds, and ends with a typed
//! confirmation that it runs as them. Only [`Installs::confirm`], given the
//! extension's id typed back, runs its build and puts it in place.
//!
//! Everything here blocks (git, the build); a daemon calls it off its runtime.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use tracing::info;

use crate::gate::{enforced, CORE_METHODS};
use crate::manifest::{file, Manifest, Permission};
use crate::pack::{
    directory_bytes, is_safe_id, is_safe_version, read_dir_manifest, InstalledPack, PackStore,
    CURRENT_FILE,
};

/// Long enough to read a sheet, short enough that walking away costs nothing.
const STAGED_TTL: Duration = Duration::from_secs(15 * 60);
/// A clone that takes longer is a source that is not answering.
const CLONE_TIMEOUT: Duration = Duration::from_secs(300);
/// `npm ci` on a slow network.
const BUILD_TIMEOUT: Duration = Duration::from_secs(900);
/// What a failure shows of a command's output.
const TAIL_BYTES: usize = 2000;
/// Left out of a bundle: history, dependencies and build output a build makes again.
const NOT_PACKED: [&str; 3] = [".git", "node_modules", "target"];

/// A command an extension runs, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SheetCommand {
    /// `build`, `service`, `action <id>` or `checks`.
    pub when: String,
    pub argv: Vec<String>,
}

/// What a person reads before anything of an extension runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    /// Confirms this sheet, and only it.
    pub token: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: Value,
    pub commands: Vec<SheetCommand>,
    /// Hosts `net.fetch` may reach for it.
    pub hosts: Vec<String>,
    /// Connection settings whose addresses it may reach too, by name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hosts_from: Vec<String>,
    /// Its declared uses, each with the Vorn methods it opens.
    pub methods: Vec<String>,
    /// What it adds to Vorn, each by name.
    pub adds: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// The sign-in it borrows from a program of the person's, in words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borrows: Option<String>,
    /// The version it replaces, when one is installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    /// The parts the bridge holds the extension to: `hosts`, `methods` and `credential`.
    pub enforced: Vec<&'static str>,
    /// The parts it only declares, which Vorn shows but cannot hold it to: `commands` and `adds`.
    pub declared: Vec<&'static str>,
    /// The words the sheet ends with.
    pub runs_as_you: String,
    /// What the person types to confirm.
    pub confirm: String,
}

#[derive(Debug)]
struct Staged {
    dir: PathBuf,
    manifest: Manifest,
    source: Value,
    expires: Instant,
}

/// The program git is and the environment git and builds run with, read at
/// each use: a daemon's own environment can still be settling when it starts.
pub type Setting = Box<dyn Fn() -> (PathBuf, Vec<(String, String)>) + Send + Sync>;

/// Installs, links and removes, in one package store.
pub struct Installs {
    store: PackStore,
    setting: Setting,
    staged: Mutex<HashMap<String, Staged>>,
}

impl std::fmt::Debug for Installs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Installs")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn token() -> String {
    crate::token::mint().unwrap_or_else(|_| format!("{:x}", now_ms()))
}

/// A git source a clone may be asked for: no option, no transport helper.
pub fn usable_git_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("A git URL is required".into());
    }
    if url.starts_with('-') || url.contains("::") || url.chars().any(char::is_control) {
        return Err(format!("{url} is not a git URL Vorn installs from"));
    }
    let remote = ["https://", "ssh://", "git@", "file://"]
        .iter()
        .any(|p| url.starts_with(p));
    if remote || Path::new(url).is_absolute() {
        Ok(())
    } else {
        Err(format!(
            "{url} is not a git URL Vorn installs from: use https://, ssh://, git@ or file://"
        ))
    }
}

/// The last bytes of a command's output, for a failure's message.
fn tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    let start = text.len().saturating_sub(TAIL_BYTES);
    let start = (start..=text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    text[start..].to_owned()
}

/// Runs `command` to its end or `timeout`, its output kept for a failure.
fn run(mut command: Command, timeout: Duration) -> Result<(ExitStatus, Vec<u8>), String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let (out, err) = (child.stdout.take(), child.stderr.take());
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = drain(out.map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = drain(err.map(|p| Box::new(p) as Box<dyn Read + Send>));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("did not finish within {} s", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut output = out.join().unwrap_or_default();
    output.extend(err.join().unwrap_or_default());
    Ok((status, output))
}

impl Installs {
    /// `git` is the program to clone with; `env` what it and builds run with.
    pub fn new(store: PackStore, git: impl Into<PathBuf>, env: Vec<(String, String)>) -> Installs {
        let git = git.into();
        Installs::reading(store, Box::new(move || (git.clone(), env.clone())))
    }

    /// As [`Installs::new`], git and its environment read at each use.
    pub fn reading(store: PackStore, setting: Setting) -> Installs {
        Installs {
            store,
            setting,
            staged: Mutex::default(),
        }
    }

    pub fn store(&self) -> &PackStore {
        &self.store
    }

    fn command(&self, program: impl AsRef<std::ffi::OsStr>, cwd: &Path) -> Command {
        let (_, env) = (self.setting)();
        let mut command = vorn_spawn::command(program);
        command
            .current_dir(cwd)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)));
        command
    }

    fn git(&self) -> PathBuf {
        (self.setting)().0
    }

    fn sweep(&self) {
        let now = Instant::now();
        lock(&self.staged).retain(|_, s| {
            let keep = s.expires > now;
            if !keep {
                let _ = fs::remove_dir_all(&s.dir);
            }
            keep
        });
    }

    /// Clones `url`, or copies a folder that is not a repository, and
    /// describes what it would install, running nothing of it.
    pub fn inspect_git(&self, url: &str, branch: Option<&str>) -> Result<Sheet, String> {
        self.inspect_git_at(url, branch, None)
    }

    /// [`Installs::inspect_git`] for the package in folder `path` of the
    /// repository, as a catalog names one of several kept together.
    pub fn inspect_git_at(
        &self,
        url: &str,
        branch: Option<&str>,
        path: Option<&str>,
    ) -> Result<Sheet, String> {
        self.sweep();
        let url = url.trim();
        usable_git_url(url)?;
        if branch.is_some_and(|b| b.starts_with('-') || b.trim().is_empty()) {
            return Err("That is not a branch or tag to install".into());
        }
        let path = path.map(str::trim).filter(|p| !p.is_empty());
        if let Some(p) = path {
            usable_subfolder(p)?;
        }
        let root = self.store.root();
        fs::create_dir_all(root).map_err(|e| format!("cannot create {}: {e}", root.display()))?;
        let dir = root.join(format!(".staging-{}", token()));
        let folder = Path::new(url);
        let plain = folder.is_dir() && !folder.join(".git").exists();
        let copied = if plain {
            copy_folder(folder, &dir).map(|()| json!({ "path": url }))
        } else {
            self.clone_into(url, branch, &dir).and_then(|rev| {
                let mut source = json!({ "git": url, "rev": rev });
                if let Some(p) = path {
                    keep_only(&dir, p)?;
                    source["path"] = json!(p);
                }
                Ok(source)
            })
        };
        let staged = copied.and_then(|source| {
            let manifest = read_staged_manifest(&dir)?;
            Ok((source, manifest))
        });
        let (source, manifest) = match staged {
            Ok(found) => found,
            Err(err) => {
                let _ = fs::remove_dir_all(&dir);
                return Err(err);
            }
        };
        let token = token();
        let sheet = self.sheet(&token, &manifest, &source);
        lock(&self.staged).insert(
            token,
            Staged {
                dir,
                manifest,
                source,
                expires: Instant::now() + STAGED_TTL,
            },
        );
        Ok(sheet)
    }

    /// A shallow clone with hooks, submodules and LFS downloads off; the
    /// history is dropped after, keeping only the commit it was.
    fn clone_into(&self, url: &str, branch: Option<&str>, dir: &Path) -> Result<String, String> {
        let parent = dir.parent().unwrap_or(dir);
        // A hooks directory that does not exist, so no template hook runs on checkout.
        let no_hooks = dir.with_extension("no-hooks");
        let mut clone = self.command(self.git(), parent);
        clone
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_LFS_SKIP_SMUDGE", "1")
            .arg("-c")
            .arg(format!("core.hooksPath={}", no_hooks.display()))
            .args(["-c", "protocol.ext.allow=never"])
            .args([
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--no-recurse-submodules",
            ]);
        if let Some(branch) = branch {
            clone.args(["--branch", branch]);
        }
        clone.arg("--").arg(url).arg(dir);
        let (status, output) =
            run(clone, CLONE_TIMEOUT).map_err(|e| format!("git clone {url} {e}"))?;
        if !status.success() {
            return Err(format!("git clone {url} failed: {}", tail(&output)));
        }
        let mut head = self.command(self.git(), dir);
        head.args(["rev-parse", "HEAD"]);
        let (status, output) = run(head, CLONE_TIMEOUT)?;
        let rev = String::from_utf8_lossy(&output).trim().to_owned();
        if !status.success() || rev.is_empty() {
            return Err(format!("{url} has no commit to install"));
        }
        fs::remove_dir_all(dir.join(".git")).map_err(|e| e.to_string())?;
        Ok(rev)
    }

    fn sheet(&self, token: &str, m: &Manifest, source: &Value) -> Sheet {
        let mut commands = Vec::new();
        let mut command = |when: String, argv: &[String]| {
            commands.push(SheetCommand {
                when,
                argv: argv.to_vec(),
            });
        };
        if let Some(build) = &m.build {
            command("build".into(), build);
        }
        if let Some(service) = &m.service {
            match &service.component {
                Some(c) => command(
                    "service, a sandboxed component".into(),
                    std::slice::from_ref(c),
                ),
                None => command("service".into(), &service.command),
            }
        }
        for run in &m.runs {
            command(format!("action {}", run.id), &run.command);
        }
        if let Some(checks) = &m.checks {
            command("checks".into(), checks);
        }
        let methods = m
            .declares
            .uses
            .iter()
            .map(|used| {
                let Some(p) = Permission::named(used) else {
                    return used.clone();
                };
                let opens: Vec<&str> = CORE_METHODS
                    .iter()
                    .filter(|(_, needs)| *needs == p)
                    .map(|(method, _)| *method)
                    .collect();
                if p == Permission::Storage {
                    format!("{used} (its own tables, storage.*)")
                } else if !enforced(p) {
                    format!("{used} (declared; Vorn cannot hold it to this)")
                } else {
                    format!("{used} ({})", opens.join(", "))
                }
            })
            .collect();
        let mut adds = Vec::new();
        for run in &m.runs {
            adds.push(format!("Action: {}", run.title));
        }
        if !m.triggers.is_empty() || !m.actions.is_empty() {
            adds.push(format!("Connector: {}", m.name));
        }
        for t in &m.triggers {
            adds.push(format!("Trigger: {}", t.label));
        }
        for a in &m.actions {
            adds.push(format!("Connector action: {}", a.label));
        }
        if let Some(c) = &m.contributes {
            adds.extend(c.panes().iter().map(|p| format!("Pane: {}", p.base.title)));
            adds.extend(
                c.footers()
                    .iter()
                    .map(|f| format!("Card footer: {}", f.base.title)),
            );
            adds.extend(
                c.link_handlers()
                    .iter()
                    .map(|l| format!("Link handler: {}", l.base.title)),
            );
            adds.extend(
                c.layouts()
                    .iter()
                    .map(|l| format!("Layout: {}", l.base.title)),
            );
            for (place, points) in &c.points {
                adds.extend(
                    points
                        .iter()
                        .map(|p| format!("{}: {}", place.label(), p.base.title)),
                );
            }
        }
        if let Some(service) = &m.service {
            adds.extend(service.events.iter().map(|e| format!("Listens to: {e}")));
        }
        adds.extend(m.mcp.iter().map(|t| format!("MCP tool: {}", t.name)));
        if m.storage.is_some() {
            adds.push("Tables of its own in Vorn's database".to_owned());
        }
        let installed = self.store.describe(&m.id);
        Sheet {
            token: token.to_owned(),
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            publisher: m
                .publisher
                .clone()
                .unwrap_or_else(|| "not named".to_owned()),
            description: m.description.clone(),
            source: source.clone(),
            commands,
            hosts: m.declares.network.clone(),
            hosts_from: m.declares.network_from.clone(),
            methods,
            adds,
            credential: m.credential.as_ref().map(|c| {
                format!(
                    "Vorn adds the connection's {} to requests to {}; {} never sees it",
                    c.field,
                    m.declares.network.join(", "),
                    m.name
                )
            }),
            borrows: borrowed(m),
            replaces: installed.map(|p| p.version),
            enforced: vec!["hosts", "methods", "credential"],
            declared: vec!["commands", "adds"],
            runs_as_you: format!(
                "{} runs as you. Its commands can read and change your files and reach the network like any program you start. Vorn holds it only to the hosts and methods above when it asks through Vorn.",
                m.name
            ),
            confirm: m.id.clone(),
        }
    }

    /// The manifest sheet `token` describes, while it can still be confirmed.
    pub fn staged_manifest(&self, token: &str) -> Option<Manifest> {
        self.sweep();
        lock(&self.staged).get(token).map(|s| s.manifest.clone())
    }

    /// Installs what sheet `token` described once `typed` is the extension's
    /// id: runs its build, then puts it in place.
    pub fn confirm(&self, token: &str, typed: &str) -> Result<InstalledPack, String> {
        self.sweep();
        let staged = lock(&self.staged).remove(token).ok_or_else(|| {
            "That install sheet has expired or was used; inspect the extension again".to_owned()
        })?;
        if typed.trim() != staged.manifest.id {
            let id = staged.manifest.id.clone();
            lock(&self.staged).insert(token.to_owned(), staged);
            return Err(format!(
                "Type {id} to confirm that it runs as you; nothing was installed"
            ));
        }
        let result = self.place(&staged);
        if result.is_err() {
            let _ = fs::remove_dir_all(&staged.dir);
        }
        result
    }

    fn place(&self, staged: &Staged) -> Result<InstalledPack, String> {
        let m = &staged.manifest;
        if let Some(build) = &m.build {
            let (program, args) = build
                .split_first()
                .ok_or_else(|| format!("{} has an empty build command", m.name))?;
            let mut command = self.command(program, &staged.dir);
            command.args(args);
            info!("[extensions] building {}: {}", m.id, build.join(" "));
            let (status, output) = run(command, BUILD_TIMEOUT)
                .map_err(|e| format!("{} could not build: {} {e}", m.name, build.join(" ")))?;
            if !status.success() {
                return Err(format!(
                    "{} did not build ({}): {}",
                    m.name,
                    build.join(" "),
                    tail(&output)
                ));
            }
        }
        let home = self.store.root().join(&m.id);
        fs::create_dir_all(&home).map_err(|e| e.to_string())?;
        let current = read_current(&home);
        let target = home.join(&m.version);
        if target.exists() {
            fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
        }
        fs::rename(&staged.dir, &target).map_err(|e| e.to_string())?;
        let previous = current
            .as_ref()
            .and_then(|c| c.get("version"))
            .and_then(Value::as_str)
            .filter(|v| *v != m.version && is_safe_version(v))
            .map(str::to_owned);
        write_current(
            &home,
            &json!({
                "version": m.version,
                "previousVersion": previous,
                "installedAt": now_ms(),
                "source": staged.source,
            }),
        )?;
        prune(&home, &[Some(m.version.as_str()), previous.as_deref()]);
        info!("[extensions] installed {} {}", m.id, m.version);
        self.store.inspect(&m.id)
    }

    /// Puts back the version an install replaced, keeping the one it
    /// replaces as the version to go back to.
    pub fn rollback(&self, id: &str) -> Result<InstalledPack, String> {
        if !is_safe_id(id) {
            return Err(format!("\"{id}\" is not an extension id"));
        }
        let home = self.store.root().join(id);
        let current =
            read_current(&home).ok_or_else(|| format!("No extension \"{id}\" is installed"))?;
        let text = |k: &str| {
            current
                .get(k)
                .and_then(Value::as_str)
                .filter(|v| is_safe_version(v))
        };
        let (Some(version), Some(previous)) = (text("version"), text("previousVersion")) else {
            return Err(format!("{id} has no earlier version to go back to"));
        };
        if !home.join(previous).is_dir() {
            return Err(format!("{id} {previous} is no longer kept"));
        }
        write_current(
            &home,
            &json!({
                "version": previous,
                "previousVersion": version,
                "installedAt": now_ms(),
                "source": current.get("source").cloned().unwrap_or(Value::Null),
            }),
        )?;
        info!("[extensions] rolled {id} back to {previous}");
        self.store.inspect(id)
    }

    /// Describes what linking `folder` would run, reading it where it is;
    /// nothing is staged, so the sheet carries no token.
    pub fn inspect_link(&self, folder: &Path) -> Result<Sheet, String> {
        let (folder, manifest) = linkable(folder)?;
        Ok(self.sheet("", &manifest, &json!({ "linked": folder })))
    }

    /// Runs `folder` in place as its manifest's extension; edits are live.
    pub fn link(&self, folder: &Path) -> Result<InstalledPack, String> {
        let (folder, manifest) = linkable(folder)?;
        let home = self.store.root().join(&manifest.id);
        if let Some(current) = read_current(&home) {
            if current.get("linked").and_then(Value::as_str).map(Path::new) != Some(&folder) {
                return Err(format!(
                    "{} is already installed; remove it before linking a folder in its place",
                    manifest.id
                ));
            }
        }
        fs::create_dir_all(&home).map_err(|e| e.to_string())?;
        write_current(&home, &json!({ "linked": folder, "installedAt": now_ms() }))?;
        info!(
            "[extensions] linked {} from {}",
            manifest.id,
            folder.display()
        );
        self.store.inspect(&manifest.id)
    }

    /// Removes extension `id` and every file Vorn keeps of it; a linked
    /// folder is the person's and stays. Its processes and rows are the
    /// caller's to stop and drop first.
    pub fn remove(&self, id: &str) -> Result<(), String> {
        if !is_safe_id(id) {
            return Err(format!("\"{id}\" is not an extension id"));
        }
        let home = self.store.root().join(id);
        if !home.join(CURRENT_FILE).exists() {
            return Err(format!("No extension \"{id}\" is installed"));
        }
        fs::remove_dir_all(&home).map_err(|e| format!("cannot remove {id}: {e}"))?;
        info!("[extensions] removed {id}");
        Ok(())
    }
}

/// What a connector borrows of the person's own sign-in, said plainly.
pub fn borrowed(m: &Manifest) -> Option<String> {
    let auth = m.auth.as_ref()?;
    let borrow = auth.borrow.as_ref()?;
    let program = auth.probe.as_ref().map(|p| p.command.as_str());
    let mut said = Vec::new();
    if let (Some(program), Some(args)) = (program, &borrow.token_args) {
        let into = borrow
            .token_env
            .as_deref()
            .map_or_else(String::new, |e| format!(" as {e}"));
        said.push(format!(
            "your {program} sign-in: Vorn runs `{program} {}` and hands it the token{into}",
            args.join(" ")
        ));
    }
    if let Some(env) = borrow.env.as_ref().filter(|e| !e.is_empty()) {
        said.push(format!("{} from your environment", env.join(", ")));
    }
    (!said.is_empty()).then(|| format!("{} borrows {}", m.name, said.join(", and ")))
}

/// A folder inside a repository: relative, plain names, never climbing out.
fn usable_subfolder(path: &str) -> Result<(), String> {
    let plain = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
    };
    if path.starts_with('/') || !path.split('/').all(plain) {
        return Err(format!("{path} is not a folder of the repository"));
    }
    Ok(())
}

/// Leaves only `path` of the clone at `dir`, moved up to be all of it.
fn keep_only(dir: &Path, path: &str) -> Result<(), String> {
    // A committed link could name a folder anywhere; each part must be a real folder of the clone.
    let mut inner = dir.to_path_buf();
    for part in path.split('/') {
        inner.push(part);
        match fs::symlink_metadata(&inner) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "{path} is a link in the repository, not a folder of it"
                ))
            }
            _ => return Err(format!("The repository has no folder {path}")),
        }
    }
    let kept = dir.with_extension("kept");
    fs::rename(&inner, &kept).map_err(|e| e.to_string())?;
    fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    fs::rename(&kept, dir).map_err(|e| e.to_string())
}

/// `folder` resolved, and the manifest it would be linked as.
fn linkable(folder: &Path) -> Result<(PathBuf, Manifest), String> {
    let folder = folder
        .canonicalize()
        .map_err(|e| format!("cannot link {}: {e}", folder.display()))?;
    let manifest = read_dir_manifest(&folder)?;
    if !is_safe_id(&manifest.id) {
        return Err(format!("\"{}\" is not an extension id", manifest.id));
    }
    Ok((folder, manifest))
}

fn read_staged_manifest(dir: &Path) -> Result<Manifest, String> {
    if !dir.join(file::FILE).is_file() {
        return Err(format!("The repository has no {} at its root", file::FILE));
    }
    crate::pack::relabel_dir(dir)?;
    let manifest = read_dir_manifest(dir)?;
    if !is_safe_id(&manifest.id) {
        return Err(format!("\"{}\" is not an extension id", manifest.id));
    }
    Ok(manifest)
}

fn read_current(home: &Path) -> Option<Value> {
    let text = fs::read_to_string(home.join(CURRENT_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_current(home: &Path, current: &Value) -> Result<(), String> {
    let tmp = home.join(format!("{CURRENT_FILE}.tmp"));
    fs::write(&tmp, current.to_string()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, home.join(CURRENT_FILE)).map_err(|e| e.to_string())
}

/// Removes every version directory but the ones kept.
fn prune(home: &Path, keep: &[Option<&str>]) {
    let Ok(entries) = fs::read_dir(home) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if is_dir && !keep.contains(&Some(name.as_str())) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// What `vorn ext pack` made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Packed {
    pub id: String,
    pub version: String,
    /// The bundle's files, relative to the folder.
    pub files: Vec<String>,
    pub bytes: u64,
    /// The recorded checks' last output, when the manifest names checks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<String>,
}

/// Records the agents a manifest's `agent` rules may name outside a server:
/// every shipped adapter's, and the ids in `installed`.
pub fn know_agents(installed: &[String]) {
    for shipped in vorn_adapters::shipped() {
        // A shipped adapter that does not read is a build fault the adapters' own tests catch.
        let _ = vorn_agents::from_manifest_text(shipped.manifest);
    }
    for id in installed {
        vorn_agents::know(id);
    }
}

/// Validates `folder`'s manifest, runs its recorded checks, and lists what a
/// bundle of it holds, writing the bundle to `out` as a `.tar.gz` when given.
/// Agent ids in `installed` count as known beside the shipped adapters'.
pub fn pack(
    folder: &Path,
    env: &[(String, String)],
    out: Option<&Path>,
    installed: &[String],
) -> Result<Packed, String> {
    know_agents(installed);
    let manifest = read_dir_manifest(folder)?;
    let checks = match &manifest.checks {
        Some(argv) => {
            let (program, args) = argv
                .split_first()
                .ok_or_else(|| "checks.command is empty".to_owned())?;
            let mut command = vorn_spawn::command(program);
            command
                .args(args)
                .current_dir(folder)
                .env_clear()
                .envs(env.iter().map(|(k, v)| (k, v)));
            let (status, output) = run(command, BUILD_TIMEOUT)
                .map_err(|e| format!("the recorded checks did not run: {e}"))?;
            if !status.success() {
                return Err(format!("the recorded checks failed: {}", tail(&output)));
            }
            Some(tail(&output))
        }
        None => None,
    };
    let mut files = Vec::new();
    walk(folder, "", &mut files).map_err(|e| e.to_string())?;
    files.sort();
    if let Some(out) = out {
        bundle(
            folder,
            &files,
            &format!("{}-{}", manifest.id, manifest.version),
            out,
        )
        .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    }
    let bytes = files
        .iter()
        .filter_map(|f| fs::metadata(folder.join(f)).ok())
        .map(|m| m.len())
        .sum();
    Ok(Packed {
        id: manifest.id,
        version: manifest.version,
        files,
        bytes,
        checks,
    })
}

/// `files` of `folder` under `top/`, gzipped.
fn bundle(folder: &Path, files: &[String], top: &str, out: &Path) -> std::io::Result<()> {
    let file = fs::File::create(out)?;
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    for f in files {
        tar.append_path_with_name(folder.join(f), format!("{top}/{f}"))?;
    }
    tar.into_inner()?.finish()?.flush()
}

/// What `pack` would bundle of `folder`, copied into `dir`.
fn copy_folder(folder: &Path, dir: &Path) -> Result<(), String> {
    let copy = || -> std::io::Result<()> {
        let mut files = Vec::new();
        walk(folder, "", &mut files)?;
        for f in &files {
            let to = dir.join(f);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(folder.join(f), to)?;
        }
        Ok(())
    };
    copy().map_err(|e| format!("cannot copy {}: {e}", folder.display()))
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if NOT_PACKED.contains(&name.as_str()) {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if entry.file_type()?.is_dir() {
            walk(&entry.path(), &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

/// The bytes Vorn keeps of an installed extension, for `vorn ext list`.
pub fn kept_bytes(pack: &InstalledPack) -> u64 {
    if pack.linked {
        0
    } else {
        directory_bytes(&pack.path).unwrap_or(0)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn git() -> Option<PathBuf> {
        let ok = Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !ok {
            eprintln!("skipped: no git on PATH");
        }
        ok.then(|| PathBuf::from("git"))
    }

    fn env() -> Vec<(String, String)> {
        std::env::vars()
            .filter(|(k, _)| k == "PATH" || k == "HOME" || k == "TMPDIR")
            .collect()
    }

    /// A repository holding `files`, committed.
    fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let p = dir.path().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "--quiet", "--initial-branch=main"]);
        git(&["add", "-A"]);
        git(&["commit", "--quiet", "-m", "init"]);
        dir
    }

    const MANIFEST: &str = r#"id = "test.marker"
name = "Marker"
version = "1.0.0"
api = "vorn:extension@1.0"
publisher = "test"

[build]
command = ["sh", "build.sh"]

[actions.hello]
title = "Say hello"
command = ["sh", "hello.sh"]

[declares]
network = ["api.example.com"]
uses = ["sessions.read", "process.run", "storage"]

[storage]
migrations = "migrations"
"#;

    #[test]
    fn runs_nothing_before_the_typed_confirmation() {
        let Some(git) = git() else { return };
        let marker = tempfile::tempdir().unwrap();
        let built = marker.path().join("built");
        let source = repo(&[
            (file::FILE, MANIFEST),
            ("build.sh", &format!("echo built > '{}'\n", built.display())),
            ("hello.sh", "cat >/dev/null\n"),
        ]);
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), git, env());
        let url = format!("file://{}", source.path().display());
        let sheet = installs.inspect_git(&url, None).unwrap();
        assert!(!built.exists(), "the build ran before the confirmation");
        assert_eq!(sheet.publisher, "test");
        assert_eq!(sheet.confirm, "test.marker");
        assert_eq!(sheet.hosts, ["api.example.com"]);
        assert_eq!(
            sheet.methods,
            [
                "sessions.read (sessions.list, sessions.get, agents.list)",
                "process.run (declared; Vorn cannot hold it to this)",
                "storage (its own tables, storage.*)"
            ]
        );
        assert_eq!(sheet.enforced, ["hosts", "methods", "credential"]);
        assert_eq!(sheet.declared, ["commands", "adds"]);
        assert_eq!(
            sheet.adds,
            ["Action: Say hello", "Tables of its own in Vorn's database"]
        );
        let whens: Vec<&str> = sheet.commands.iter().map(|c| c.when.as_str()).collect();
        assert_eq!(whens, ["build", "action hello"]);
        assert!(sheet.runs_as_you.starts_with("Marker runs as you."));
        assert!(installs.store().describe("test.marker").is_none());

        let wrong = installs.confirm(&sheet.token, "yes").unwrap_err();
        assert!(wrong.starts_with("Type test.marker to confirm"), "{wrong}");
        assert!(!built.exists());

        let pack = installs.confirm(&sheet.token, "test.marker").unwrap();
        assert!(built.exists(), "the build runs once confirmed");
        assert_eq!(pack.version, "1.0.0");
        assert_eq!(pack.source.as_ref().unwrap()["git"], url.as_str());
        assert!(!pack.path.join(".git").exists());
        assert!(installs.confirm(&sheet.token, "test.marker").is_err());

        installs.remove("test.marker").unwrap();
        assert!(!root.path().join("test.marker").exists());
        let left: Vec<_> = fs::read_dir(root.path()).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn installs_a_folder_that_is_not_a_repository_as_a_copy() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join(file::FILE),
            MANIFEST.replace("[build]\ncommand = [\"sh\", \"build.sh\"]\n", ""),
        )
        .unwrap();
        fs::write(folder.path().join("hello.sh"), "").unwrap();
        fs::create_dir_all(folder.path().join("node_modules/x")).unwrap();
        fs::write(folder.path().join("node_modules/x/i.js"), "").unwrap();
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), "git", env());
        let path = folder.path().display().to_string();
        let sheet = installs.inspect_git(&path, None).unwrap();
        assert_eq!(sheet.source, json!({ "path": path }));
        let pack = installs.confirm(&sheet.token, "test.marker").unwrap();
        assert!(!pack.linked);
        assert!(pack.path.join("hello.sh").exists());
        assert!(!pack.path.join("node_modules").exists());
        fs::write(folder.path().join("hello.sh"), "echo edited").unwrap();
        assert_eq!(fs::read_to_string(pack.path.join("hello.sh")).unwrap(), "");
    }

    #[test]
    fn install_and_link_check_the_api_and_vorn_range_against_this_vorn() {
        let both = |from: &str, to: &str| {
            let folder = tempfile::tempdir().unwrap();
            let text = MANIFEST
                .replace("[build]\ncommand = [\"sh\", \"build.sh\"]\n", "")
                .replace(from, to);
            fs::write(folder.path().join(file::FILE), text).unwrap();
            fs::write(folder.path().join("hello.sh"), "").unwrap();
            let root = tempfile::tempdir().unwrap();
            let installs = Installs::new(PackStore::new(root.path()), "git", env());
            let path = folder.path().display().to_string();
            let copied = installs.inspect_git(&path, None).map(|_| ());
            let linked = installs.inspect_link(folder.path()).map(|_| ());
            (copied, linked)
        };
        let attempt = |from: &str, to: &str| {
            let (copied, linked) = both(from, to);
            assert_eq!(copied, linked);
            copied
        };
        let api = "api = \"vorn:extension@1.0\"";
        // Equal.
        assert_eq!(attempt(api, api), Ok(()));
        assert_eq!(attempt(api, &format!("{api}\nvorn = \">=1.0\"")), Ok(()));
        // Newer.
        let newer = attempt(api, "api = \"vorn:extension@1.1\"").unwrap_err();
        assert_eq!(
            newer,
            "vorn-extension.toml:4: api: built for vorn:extension@1.1; this Vorn speaks vorn:extension@1.0, so update Vorn to run it"
        );
        let newer = attempt(api, &format!("{api}\nvorn = \">=1.1\"")).unwrap_err();
        assert_eq!(
            newer,
            "vorn-extension.toml:5: vorn: runs on vorn:extension >=1.1; this Vorn speaks vorn:extension@1.0, so update Vorn to run it"
        );
        // Older.
        let older = attempt(api, "api = \"vorn:extension@0.8\"").unwrap_err();
        assert!(
            older.contains("built for vorn:extension@0.8, which this Vorn no longer speaks; it speaks vorn:extension@1.0"),
            "{older}"
        );
        let older = attempt(api, &format!("{api}\nvorn = \"<1.0\"")).unwrap_err();
        assert!(older.contains("update the extension"), "{older}");
        // A 0.9 package is relabelled as it installs; a linked folder is not.
        let (copied, linked) = both(api, "api = \"vorn:extension@0.9\"");
        assert_eq!(copied, Ok(()));
        assert!(linked.unwrap_err().contains("built for vorn:extension@0.9"));
    }

    #[test]
    fn a_failed_build_installs_nothing() {
        let Some(git) = git() else { return };
        let source = repo(&[
            (file::FILE, MANIFEST),
            ("build.sh", "echo 'npm ERR! offline' >&2; exit 1\n"),
            ("hello.sh", ""),
        ]);
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), git, env());
        let sheet = installs
            .inspect_git(&source.path().display().to_string(), None)
            .unwrap();
        let err = installs.confirm(&sheet.token, "test.marker").unwrap_err();
        assert!(err.contains("npm ERR! offline"), "{err}");
        let left: Vec<_> = fs::read_dir(root.path()).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn refuses_a_repository_without_a_manifest_and_urls_that_are_options() {
        let Some(git) = git() else { return };
        let source = repo(&[("README.md", "hi")]);
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), git, env());
        let err = installs
            .inspect_git(&source.path().display().to_string(), None)
            .unwrap_err();
        assert_eq!(err, "The repository has no vorn-extension.toml at its root");
        for url in ["--upload-pack=x", "ext::sh -c x", "relative/dir", ""] {
            assert!(usable_git_url(url).is_err(), "{url}");
        }
        let left: Vec<_> = fs::read_dir(root.path()).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn links_a_folder_and_packs_it() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join(file::FILE),
            MANIFEST.replace(
                "[build]\ncommand = [\"sh\", \"build.sh\"]\n",
                "[checks]\ncommand = [\"sh\", \"-c\", \"echo 3 checks passed\"]\n",
            ),
        )
        .unwrap();
        fs::write(folder.path().join("hello.sh"), "").unwrap();
        fs::create_dir_all(folder.path().join("node_modules/x")).unwrap();
        fs::write(folder.path().join("node_modules/x/i.js"), "").unwrap();
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), "git", env());
        let sheet = installs.inspect_link(folder.path()).unwrap();
        assert!(sheet.token.is_empty());
        assert_eq!(sheet.confirm, "test.marker");
        let canonical = folder.path().canonicalize().unwrap();
        assert_eq!(sheet.source, json!({ "linked": canonical }));
        assert!(installs.store.describe("test.marker").is_none());
        let pack = installs.link(folder.path()).unwrap();
        assert!(pack.linked);
        // Linking the same folder again is a no-op; another id in its place is not.
        installs.link(folder.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        let bundle = out.path().join("marker.tar.gz");
        let packed = super::pack(folder.path(), &env(), Some(&bundle), &[]).unwrap();
        assert_eq!(packed.files, ["hello.sh", "vorn-extension.toml"]);
        let mut names: Vec<String> = tar::Archive::new(flate2::read::GzDecoder::new(
            fs::File::open(&bundle).unwrap(),
        ))
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
        .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "test.marker-1.0.0/hello.sh",
                "test.marker-1.0.0/vorn-extension.toml"
            ]
        );
        assert_eq!(packed.checks.as_deref(), Some("3 checks passed"));
        installs.remove("test.marker").unwrap();
        assert!(
            folder.path().join("hello.sh").exists(),
            "a linked folder stays"
        );
    }

    #[test]
    fn packs_a_manifest_naming_a_shipped_or_installed_agent() {
        let folder = tempfile::tempdir().unwrap();
        let manifest = |agent: &str| {
            format!(
                "id = \"test.agents\"\nname = \"Agents\"\nversion = \"1.0.0\"\napi = \"vorn:extension@1.0\"\npublisher = \"test\"\n\n[actions.hello]\ntitle = \"Hello\"\ncommand = [\"sh\", \"hello.sh\"]\n\n[activates]\nagent = [\"{agent}\"]\n"
            )
        };
        let write =
            |agent: &str| fs::write(folder.path().join(file::FILE), manifest(agent)).unwrap();
        write("claude");
        super::pack(folder.path(), &env(), None, &[]).unwrap();
        write("acme-pack-test");
        assert!(super::pack(folder.path(), &env(), None, &[]).is_err());
        super::pack(folder.path(), &env(), None, &["acme-pack-test".to_owned()]).unwrap();
    }

    #[test]
    fn installs_one_folder_of_a_repository_and_rolls_back() {
        let Some(git) = git() else { return };
        let v = |version: &str| {
            MANIFEST
                .replace("1.0.0", version)
                .replace("[build]\ncommand = [\"sh\", \"build.sh\"]\n", "")
        };
        let source = repo(&[
            ("connectors/marker/vorn-extension.toml", &v("1.0.0")),
            ("connectors/marker/hello.sh", "cat >/dev/null\n"),
            ("connectors/other/README.md", "not this one\n"),
        ]);
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), git, env());
        let url = format!("file://{}", source.path().display());
        for bad in ["../x", "/etc", "a/../b", "a b"] {
            assert!(
                installs.inspect_git_at(&url, None, Some(bad)).is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            installs
                .inspect_git_at(&url, None, Some("connectors/none"))
                .unwrap_err(),
            "The repository has no folder connectors/none"
        );
        let sheet = installs
            .inspect_git_at(&url, None, Some("connectors/marker"))
            .unwrap();
        assert_eq!(sheet.source["path"], "connectors/marker");
        let pack = installs.confirm(&sheet.token, "test.marker").unwrap();
        assert!(pack.path.join("hello.sh").is_file());
        assert!(!pack.path.join("connectors").exists());
        assert_eq!(
            installs.rollback("test.marker").unwrap_err(),
            "test.marker has no earlier version to go back to"
        );
        fs::write(
            source.path().join("connectors/marker/vorn-extension.toml"),
            v("2.0.0"),
        )
        .unwrap();
        let commit = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qam",
                "two",
            ])
            .current_dir(source.path())
            .status()
            .unwrap();
        assert!(commit.success());
        let sheet = installs
            .inspect_git_at(&url, None, Some("connectors/marker"))
            .unwrap();
        assert_eq!(sheet.replaces.as_deref(), Some("1.0.0"));
        installs.confirm(&sheet.token, "test.marker").unwrap();
        let back = installs.rollback("test.marker").unwrap();
        assert_eq!(back.version, "1.0.0");
        assert_eq!(back.previous_version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn says_what_a_connector_borrows_and_where_its_hosts_come_from() {
        let m = file::parse(
            r#"id = "test.gh"
name = "GitHub"
version = "1.0.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "index.js"]

[declares]
network = ["api.github.com"]
network_from = ["GH_HOSTS"]

[connector.auth]
rung = "cli"
probe = { command = "gh", args = ["auth", "status"] }
borrow = { token_args = ["auth", "token"], token_env = "GH_TOKEN", env = ["GH_TOKEN", "GH_HOST"] }

[[connector.env]]
name = "GH_HOSTS"

[[connector.triggers]]
type = "issue"
"#,
        )
        .unwrap();
        assert_eq!(
            borrowed(&m).as_deref(),
            Some("GitHub borrows your gh sign-in: Vorn runs `gh auth token` and hands it the token as GH_TOKEN, and GH_TOKEN, GH_HOST from your environment")
        );
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), "git", env());
        let sheet = installs.sheet("t", &m, &json!({}));
        assert_eq!(sheet.hosts, ["api.github.com"]);
        assert_eq!(sheet.hosts_from, ["GH_HOSTS"]);
        let bad = file::parse(
            r#"id = "test.x"
name = "X"
version = "1.0.0"
api = "vorn:extension@1.0"
[service]
command = ["node", "index.js"]
[declares]
network_from = ["not a name"]
[[connector.triggers]]
type = "t"
"#,
        )
        .unwrap_err();
        assert_eq!(bad.field, "declares.network_from[0]");
    }
}

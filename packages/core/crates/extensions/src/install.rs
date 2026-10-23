//! Installing an extension from git or from a pack, linking a folder,
//! removing one, and packing one for the catalog.
//!
//! An install runs nothing of the extension's before the person confirms:
//! [`Installs::inspect_git`] clones with hooks off, and
//! [`Installs::inspect_archive`] unpacks a `.vorn.tgz`, and each reads the
//! manifest into a [`Sheet`] that names the publisher, every command, every host, the
//! Vorn methods it may call and what it adds, and ends with a typed
//! confirmation that it runs as them. Only [`Installs::confirm`], given the
//! extension's id typed back, runs its build and puts it in place.
//!
//! Everything here blocks (git, the build); a daemon calls it off its runtime.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::gate::{enforced, CORE_METHODS};
use crate::manifest::{file, Manifest, Permission};
use crate::pack::{
    check_id, directory_bytes, is_safe_id, is_safe_version, read_dir_manifest, InstalledPack,
    PackStore, CURRENT_FILE, REFUSED_KEY, SHIPPED_NAMESPACE,
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
/// Starts the lock an editor keeps beside a file it has open, a link to nothing.
const EDITOR_LOCK: &str = ".#";
/// The most a pack may unpack to; Vorn's own are well under a megabyte.
const ARCHIVE_MAX_BYTES: u64 = 64 << 20;
/// The most files a pack may hold.
const ARCHIVE_MAX_FILES: usize = 4096;

/// The pack Vorn pinned for an extension: in the shipped set the installer
/// carries, or in the official catalog. Only a pack whose bytes are these may
/// have an id in Vorn's own namespace (`vorn.`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    pub id: String,
    pub version: String,
    /// The pack's sha256, in lowercase hex.
    pub sha256: String,
}

/// What this build of Vorn vouches for, compiled into it: the packs it pins
/// (its shipped set, and Vorn's own the catalog offers), the only ones that
/// may have an id in Vorn's own namespace, and the ids of the extensions it
/// bundles, which no install may take. A catalog listing vouches for nothing
/// in Vorn's namespace: it is not signed.
#[derive(Debug, Clone, Default)]
pub struct Trust {
    pub pins: Vec<Pinned>,
    pub bundled: BTreeSet<String>,
}

impl Trust {
    fn vouches(&self, pin: &Pinned) -> bool {
        self.pins.contains(pin)
    }

    /// Whether `pack` is installed as the very pack this build pins for its
    /// id and version, which makes it Vorn's own: never a linked folder, and
    /// only while its files are still exactly the pinned pack's. Blocks on the disk.
    pub fn vouches_for(&self, pack: &InstalledPack) -> bool {
        !pack.linked
            && pin_of(&pack.id, &pack.version, pack.source.as_ref()).is_some_and(|pin| {
                self.vouches(&pin) && pinned_files(&pack.path, &pin.sha256).is_some()
            })
    }

    /// `file` of `pack`, read once, and whether these very bytes are Vorn's
    /// own: `pack` is vouched for and they are the pinned pack's `file`. What
    /// runs is what was checked, so a file changed after the check runs as no one's.
    pub fn read_vouched(
        &self,
        pack: &InstalledPack,
        file: &str,
    ) -> Result<(Vec<u8>, bool), String> {
        let path = pack.path.join(file);
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let key: Vec<String> = Path::new(file)
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let vouched = !pack.linked
            && pin_of(&pack.id, &pack.version, pack.source.as_ref())
                .filter(|pin| self.vouches(pin))
                .and_then(|pin| pinned_files(&pack.path, &pin.sha256))
                .is_some_and(|files| files.get(&key.join("/")) == Some(&sha256_hex(&bytes)));
        Ok((bytes, vouched))
    }

    /// Whether only a pack this build pins may have `id`.
    fn guards(&self, id: &str) -> bool {
        id.starts_with(SHIPPED_NAMESPACE) || self.bundled.contains(id)
    }
}

/// The pinned pack's own bytes, kept in its folder so its files can be checked against the pin.
pub const PINNED_ARCHIVE: &str = ".pinned.vorn.tgz";

/// Each file of the pack kept in `dir`, with its sha256, when `dir` holds
/// exactly those files and the kept pack's sha256 is `sha256`: none
/// changed, added or missing, and no links.
fn pinned_files(dir: &Path, sha256: &str) -> Option<BTreeMap<String, String>> {
    let bytes = fs::read(dir.join(PINNED_ARCHIVE)).ok()?;
    if sha256_hex(&bytes) != sha256 {
        return None;
    }
    let packed = archive_digests(&bytes).ok()?;
    let mut installed = BTreeMap::new();
    tree_digests(dir, "", &mut installed).ok()?;
    (packed == installed).then_some(packed)
}

/// Each file of a `.vorn.tgz` under its top folder, by path, with its sha256.
fn archive_digests(bytes: &[u8]) -> std::io::Result<BTreeMap<String, String>> {
    use std::path::Component;
    let mut out = BTreeMap::new();
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(std::io::Error::other("not a plain file"));
        }
        let path = entry.path()?;
        let parts: Vec<String> = path
            .components()
            .filter_map(|c| match c {
                Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
                _ => None,
            })
            .skip(1)
            .collect();
        let mut content = Vec::new();
        entry.read_to_end(&mut content)?;
        out.insert(parts.join("/"), sha256_hex(&content));
    }
    Ok(out)
}

/// Each file under `dir`, by path, with its sha256; the kept pack itself left out.
fn tree_digests(
    dir: &Path,
    prefix: &str,
    out: &mut BTreeMap<String, String>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if prefix.is_empty() && name == PINNED_ARCHIVE {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            tree_digests(&entry.path(), &rel, out)?;
        } else if kind.is_file() {
            out.insert(rel, sha256_hex(&fs::read(entry.path())?));
        } else {
            return Err(std::io::Error::other("not a plain file"));
        }
    }
    Ok(())
}

/// `bytes`' sha256, in lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

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
    trust: Trust,
    staged: Mutex<HashMap<String, Staged>>,
    /// Held from an install's last check to its `current.json`, so two
    /// installs at once cannot both claim one agent.
    placing: Mutex<()>,
    /// The linked folders the last [`Installs::recheck_linked`] found a
    /// link in, by id: one is refused only once a second check finds it too.
    linking: Mutex<HashMap<String, String>>,
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

/// When the extension `current` points at was first installed: an update or
/// a rollback keeps its place among the others, which are ordered by it.
fn first_installed(current: Option<&Value>) -> u64 {
    current
        .and_then(|c| c.get("installedAt"))
        .and_then(Value::as_u64)
        .unwrap_or_else(now_ms)
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
            trust: Trust::default(),
            staged: Mutex::default(),
            placing: Mutex::default(),
            linking: Mutex::default(),
        }
    }

    /// These installs, admitting to Vorn's namespace only what `trust` pins.
    pub fn trusting(mut self, trust: Trust) -> Installs {
        self.trust = trust;
        self
    }

    /// What vouches for packs in Vorn's namespace, which the router lets own it too.
    pub fn trust(&self) -> &Trust {
        &self.trust
    }

    /// Refuses `pin` up front when it claims an id only this build's own
    /// pins may have and is not one of them, as a catalog entry may.
    pub fn vouch(&self, pin: &Pinned) -> Result<(), String> {
        if self.trust.guards(&pin.id) && !self.trust.vouches(pin) {
            return Err(format!(
                "{} {} is not a pack this Vorn ships, and only those may use the id {}; nothing was installed",
                pin.id, pin.version, pin.id
            ));
        }
        Ok(())
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
                refuse_links(&dir, "")?;
                Ok(source)
            })
        };
        let staged = copied.and_then(|source| {
            let manifest = self.read_staged_manifest(&dir, None)?;
            Ok((source, manifest))
        });
        self.stage(dir, staged)
    }

    /// Unpacks `bytes`, a `.vorn.tgz` with one folder at its top, and
    /// describes what it would install, running nothing of it. With `pinned`
    /// the bytes must be exactly the pinned pack's; only a pin this build
    /// trusts ([`Trust`]) lets the id be in Vorn's own namespace.
    pub fn inspect_archive(
        &self,
        bytes: &[u8],
        source: Value,
        pinned: Option<&Pinned>,
    ) -> Result<Sheet, String> {
        self.sweep();
        if let Some(pin) = pinned {
            self.vouch(pin)?;
        }
        let sha256 = sha256_hex(bytes);
        if let Some(pin) = pinned {
            if pin.sha256 != sha256 {
                return Err(format!(
                    "The pack offered as {} {} is not the one Vorn pinned (its sha256 is {sha256}, not {}); nothing was installed",
                    pin.id, pin.version, pin.sha256
                ));
            }
        }
        let root = self.store.root();
        fs::create_dir_all(root).map_err(|e| format!("cannot create {}: {e}", root.display()))?;
        let dir = root.join(format!(".staging-{}", token()));
        let vouched = pinned.is_some_and(|p| self.trust.vouches(p));
        let staged = unpack(bytes, &dir).and_then(|()| {
            if vouched {
                fs::write(dir.join(PINNED_ARCHIVE), bytes).map_err(|e| e.to_string())?;
            }
            let manifest = self.read_staged_manifest(&dir, pinned)?;
            let mut source = source;
            source["sha256"] = json!(sha256);
            Ok((source, manifest))
        });
        self.stage(dir, staged)
    }

    /// Keeps what was read into `dir` for [`Installs::confirm`], or drops it when it did not read.
    fn stage(
        &self,
        dir: PathBuf,
        staged: Result<(Value, Manifest), String>,
    ) -> Result<Sheet, String> {
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
                } else if p == Permission::ProcessRun {
                    format!(
                        "{used} (runs commands: workflow steps it seeds, workflows not its own)"
                    )
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
        match &result {
            Ok(pack) => self.shipped_record().accept(&pack.id),
            Err(_) => {
                let _ = fs::remove_dir_all(&staged.dir);
            }
        }
        result
    }

    fn shipped_record(&self) -> crate::shipped::Record {
        crate::shipped::Record::beside(self.store.root())
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
        // The build ran the pack's own code: what it leaves is checked again.
        if read_dir_manifest(&staged.dir)? != *m {
            return Err(format!(
                "{}'s build changed its manifest; nothing was installed",
                m.name
            ));
        }
        let _placing = lock(&self.placing);
        self.admit(m, pin_of(&m.id, &m.version, Some(&staged.source)).as_ref())?;
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
        let previous_source = previous
            .as_ref()
            .and_then(|_| current.as_ref()?.get("source").cloned());
        write_current(
            &home,
            &json!({
                "version": m.version,
                "previousVersion": previous,
                "installedAt": first_installed(current.as_ref()),
                "source": staged.source,
                "previousSource": previous_source,
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
        let manifest = read_dir_manifest(&home.join(previous))?;
        if manifest.id != id || manifest.version != previous {
            return Err(format!("{id} {previous} is not the version it was"));
        }
        let source = current.get("previousSource").filter(|s| !s.is_null());
        let pin = pin_of(id, previous, source);
        if self.trust.guards(id) && !pin.as_ref().is_some_and(|p| self.trust.vouches(p)) {
            return Err(format!(
                "{id} {previous} is not a pack this Vorn ships, and only those may use the id {id}; it stays at {version}"
            ));
        }
        let _placing = lock(&self.placing);
        self.admit(&manifest, pin.as_ref())?;
        write_current(
            &home,
            &json!({
                "version": previous,
                "previousVersion": version,
                "installedAt": first_installed(Some(&current)),
                "source": source.cloned().unwrap_or(Value::Null),
                "previousSource": current.get("source").cloned().unwrap_or(Value::Null),
            }),
        )?;
        info!("[extensions] rolled {id} back to {previous}");
        self.store.inspect(id)
    }

    /// Describes what linking `folder` would run, reading it where it is;
    /// nothing is staged, so the sheet carries no token.
    pub fn inspect_link(&self, folder: &Path) -> Result<Sheet, String> {
        let (folder, manifest) = self.linkable(folder)?;
        Ok(self.sheet("", &manifest, &json!({ "linked": folder })))
    }

    /// Runs `folder` in place as its manifest's extension; edits are live.
    pub fn link(&self, folder: &Path) -> Result<InstalledPack, String> {
        let _placing = lock(&self.placing);
        let (folder, manifest) = self.linkable(folder)?;
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
        let mut current = json!({ "linked": folder, "installedAt": now_ms() });
        if let Some(agent) = manifest.agent_id() {
            current["linkedAgent"] = json!(agent);
            let text = fs::read(folder.join(file::FILE)).map_err(|e| e.to_string())?;
            current["linkedManifest"] = json!(sha256_hex(&text));
        }
        write_current(&home, &current)?;
        info!(
            "[extensions] linked {} from {}",
            manifest.id,
            folder.display()
        );
        self.store.inspect(&manifest.id)
    }

    /// Checks every linked folder again as the link did, since one can gain
    /// a link or special file after it was linked: one that still holds it
    /// at the next check is marked not loaded in its `current.json` until it
    /// holds none again. Answers the ids whose mark changed. Blocks on the disk.
    pub fn recheck_linked(&self) -> Vec<String> {
        let mut changed = Vec::new();
        for id in self.store.ids() {
            let home = self.store.root().join(&id);
            let Some(folder) = read_current(&home)
                .and_then(|c| c.get("linked").and_then(Value::as_str).map(PathBuf::from))
            else {
                continue;
            };
            let found = match walk(&folder, "", &mut Vec::new()) {
                Ok(()) => None,
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Some(e.to_string()),
                // A folder that cannot be read now is not loaded anyway; its mark stays as it was.
                Err(_) => continue,
            };
            let _placing = lock(&self.placing);
            let Some(mut current) = read_current(&home) else {
                continue;
            };
            // Linked again, or removed, while the folder was walked.
            if current.get("linked").and_then(Value::as_str).map(Path::new) != Some(&folder) {
                continue;
            }
            let was = current.get(REFUSED_KEY).and_then(Value::as_str);
            let seen_before = {
                let mut linking = lock(&self.linking);
                match &found {
                    Some(why) => linking.insert(id.clone(), why.clone()).is_some(),
                    None => linking.remove(&id).is_some(),
                }
            };
            // A link seen once may be gone by the next check, as a save's is.
            if found.is_some() && was.is_none() && !seen_before {
                continue;
            }
            let refused = found;
            if was == refused.as_deref() {
                continue;
            }
            match &refused {
                Some(why) => {
                    warn!("[extensions] {id} is not loaded: {why}");
                    current[REFUSED_KEY] = json!(why);
                }
                None => {
                    info!("[extensions] {id} holds no link any more and loads again");
                    if let Some(map) = current.as_object_mut() {
                        map.remove(REFUSED_KEY);
                    }
                }
            }
            match write_current(&home, &current) {
                Ok(()) => changed.push(id),
                Err(e) => warn!("[extensions] cannot mark {id}: {e}"),
            }
        }
        changed
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
        // Vorn's own come back on the next start unless their removal is kept.
        if id.starts_with(SHIPPED_NAMESPACE) {
            self.shipped_record().decline(id);
        }
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

/// Refuses a tree holding a link or special file anywhere, as a pack holds
/// only files and folders; a clone keeps what was committed, links included.
fn refuse_links(dir: &Path, prefix: &str) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry.file_type().map_err(|e| format!("{rel}: {e}"))?;
        if kind.is_dir() {
            refuse_links(&entry.path(), &rel)?;
        } else if !kind.is_file() {
            return Err(format!(
                "{rel} is a link or special file; a pack holds only files and folders"
            ));
        }
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

impl Installs {
    /// `folder` resolved, and the manifest it would be linked as.
    fn linkable(&self, folder: &Path) -> Result<(PathBuf, Manifest), String> {
        let folder = folder
            .canonicalize()
            .map_err(|e| format!("cannot link {}: {e}", folder.display()))?;
        walk(&folder, "", &mut Vec::new()).map_err(|e| format!("cannot link: {e}"))?;
        let manifest = read_dir_manifest(&folder)?;
        self.admit(&manifest, None)?;
        Ok((folder, manifest))
    }

    fn read_staged_manifest(
        &self,
        dir: &Path,
        pinned: Option<&Pinned>,
    ) -> Result<Manifest, String> {
        if !dir.join(file::FILE).is_file() {
            return Err(format!("The repository has no {} at its root", file::FILE));
        }
        crate::pack::relabel_dir(dir)?;
        let manifest = read_dir_manifest(dir)?;
        self.admit(&manifest, pinned)?;
        Ok(manifest)
    }

    /// Whether `manifest` may be installed: under an id of its own, or under
    /// Vorn's own namespace or a bundled id only as the very pack this build
    /// pins for it; and, but for Vorn's own, as the only adapter of its agent.
    fn admit(&self, manifest: &Manifest, pinned: Option<&Pinned>) -> Result<(), String> {
        let id = &manifest.id;
        if let Some(pin) = pinned {
            if pin.id != *id || pin.version != manifest.version {
                return Err(format!(
                    "The pack pinned as {} {} holds {id} {}; nothing was installed",
                    pin.id, pin.version, manifest.version
                ));
            }
        }
        if pinned.is_some_and(|p| self.trust.vouches(p)) {
            // Vorn's own adapter wins its agent id over any other.
            return if is_safe_id(id) {
                Ok(())
            } else {
                Err(format!("\"{id}\" is not an extension id"))
            };
        }
        if self.trust.bundled.contains(id) {
            return Err(format!(
                "\"{id}\" is the id of an extension Vorn bundles; choose another id"
            ));
        }
        installable_id(id)?;
        self.agent_unclaimed(manifest)
    }

    /// Refuses an adapter for an agent another installed extension already runs.
    fn agent_unclaimed(&self, manifest: &Manifest) -> Result<(), String> {
        let Some(agent) = manifest.agent_id() else {
            return Ok(());
        };
        for other in self.store.ids().into_iter().filter(|o| *o != manifest.id) {
            let Ok(pack) = self.store.inspect_unmeasured(&other) else {
                continue;
            };
            if pack.manifest.agent_id().as_deref() == Some(agent.as_str()) {
                return Err(format!(
                    "{} adds the agent \"{agent}\", which {other} already runs. Remove {other} first, or give this adapter another agent id; nothing was installed",
                    manifest.name
                ));
            }
        }
        Ok(())
    }
}

/// Unpacks a `.vorn.tgz` into `dir`, its one top folder dropped: plain files
/// only, every path inside that folder, and no more than a pack may hold.
fn unpack(bytes: &[u8], dir: &Path) -> Result<(), String> {
    use std::path::Component;
    let bad = |why: &str| format!("That is not a pack Vorn installs: {why}");
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let entries = archive.entries().map_err(|e| bad(&e.to_string()))?;
    let (mut top, mut total, mut files) = (None::<std::ffi::OsString>, 0u64, 0usize);
    for entry in entries {
        let mut entry = entry.map_err(|e| bad(&e.to_string()))?;
        let kind = entry.header().entry_type();
        let path = entry.path().map_err(|e| bad(&e.to_string()))?.into_owned();
        let mut parts = Vec::new();
        for part in path.components() {
            match part {
                Component::Normal(p) => parts.push(p.to_owned()),
                Component::CurDir => {}
                _ => return Err(bad(&format!("{} leaves its folder", path.display()))),
            }
        }
        let Some((first, rest)) = parts.split_first() else {
            continue;
        };
        match &top {
            None => top = Some(first.clone()),
            Some(t) if t != first => return Err(bad("it holds more than one folder")),
            Some(_) => {}
        }
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() || rest.is_empty() {
            return Err(bad(&format!("{} is not a plain file", path.display())));
        }
        files += 1;
        // A PAX header may give the real size; the ustar field alone can understate it.
        total = total.saturating_add(entry.size());
        if files > ARCHIVE_MAX_FILES || total > ARCHIVE_MAX_BYTES {
            return Err(bad("it is larger than a pack may be"));
        }
        let to = rest.iter().fold(dir.to_path_buf(), |p, part| p.join(part));
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = fs::File::create(&to).map_err(|e| e.to_string())?;
        std::io::copy(&mut (&mut entry).take(ARCHIVE_MAX_BYTES), &mut out)
            .map_err(|e| bad(&e.to_string()))?;
    }
    if top.is_none() {
        return Err(bad("it is empty"));
    }
    Ok(())
}

/// Refuses what [`check_id`] refuses, and a theme Vorn ships, so no
/// extension can take the place of a bundled theme.
fn installable_id(id: &str) -> Result<(), String> {
    check_id(id)?;
    if vorn_theme::find_bundled(id).is_some() {
        return Err(format!(
            "\"{id}\" is the id of a theme Vorn ships; choose another id"
        ));
    }
    Ok(())
}

/// The pin a pack installed from `source` answers to: its archive's sha256, when it came as one.
fn pin_of(id: &str, version: &str, source: Option<&Value>) -> Option<Pinned> {
    let sha256 = source?.get("sha256")?.as_str()?;
    Some(Pinned {
        id: id.to_owned(),
        version: version.to_owned(),
        sha256: sha256.to_owned(),
    })
}

fn read_current(home: &Path) -> Option<Value> {
    let text = fs::read_to_string(home.join(CURRENT_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Replaces `current.json` atomically, through a fresh file no link can stand in for.
fn write_current(home: &Path, current: &Value) -> Result<(), String> {
    let tmp = home.join(format!("{CURRENT_FILE}.{}.tmp", token()));
    let write = || -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(current.to_string().as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, home.join(CURRENT_FILE))
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
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
/// the ids in `installed`, those of the adapters installed.
pub fn know_agents(installed: &[String]) {
    for id in installed {
        vorn_agents::know(id);
    }
}

/// Validates `folder`'s manifest, runs its recorded checks, and lists what a
/// bundle of it holds, writing the bundle to `out` as a `.tar.gz` when given.
/// Agent ids in `installed`, those of the adapters installed, count as known.
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
    tar.follow_symlinks(false);
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

/// Each file under `dir` but what is never packed, by path; a link or
/// special file anywhere is an `InvalidData` error naming it.
fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if NOT_PACKED.contains(&name.as_str()) || name.starts_with(EDITOR_LOCK) {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        // `file_type` does not follow links; a pack holds only real files and folders.
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(&entry.path(), &rel, out)?;
        } else if kind.is_file() {
            out.push(rel);
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{rel} is a link or special file; a pack holds only files and folders"),
            ));
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
                "process.run (runs commands: workflow steps it seeds, workflows not its own)",
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

    #[cfg(unix)]
    #[test]
    fn a_folder_holding_a_link_is_neither_copied_nor_packed() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private"), "kept out").unwrap();
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join(file::FILE),
            MANIFEST.replace("[build]\ncommand = [\"sh\", \"build.sh\"]\n", ""),
        )
        .unwrap();
        fs::write(folder.path().join("hello.sh"), "").unwrap();
        std::os::unix::fs::symlink(outside.path().join("private"), folder.path().join("a"))
            .unwrap();

        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), "git", env());
        let path = folder.path().display().to_string();
        let err = installs.inspect_git(&path, None).unwrap_err();
        assert!(err.contains("a is a link"), "{err}");
        let left: Vec<_> = fs::read_dir(root.path()).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");

        let out = folder.path().join("out.tar.gz");
        let err = pack(folder.path(), &env(), Some(&out), &[]).unwrap_err();
        assert!(err.contains("a is a link"), "{err}");

        fs::remove_file(folder.path().join("a")).unwrap();
        std::os::unix::fs::symlink(outside.path(), folder.path().join("dir")).unwrap();
        let err = pack(folder.path(), &env(), None, &[]).unwrap_err();
        assert!(err.contains("dir is a link"), "{err}");
    }

    #[test]
    fn a_link_committed_to_a_repository_is_refused() {
        if git().is_none() {
            return;
        }
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private"), "kept out").unwrap();
        let manifest = MANIFEST.replace("[build]\ncommand = [\"sh\", \"build.sh\"]\n", "");
        let repo = repo(&[
            (file::FILE, &manifest),
            ("sub/vorn-extension.toml", &manifest),
        ]);
        std::os::unix::fs::symlink(outside.path().join("private"), repo.path().join("sub/a"))
            .unwrap();
        let added = Command::new("git")
            .args(["add", "sub/a"])
            .current_dir(repo.path())
            .status()
            .unwrap()
            .success();
        let committed = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "link",
            ])
            .current_dir(repo.path())
            .status()
            .unwrap()
            .success();
        assert!(added && committed);
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), "git", env());
        let url = format!("file://{}", repo.path().display());
        let err = installs.inspect_git(&url, None).unwrap_err();
        assert!(err.contains("sub/a is a link"), "{err}");
        let err = installs
            .inspect_git_at(&url, None, Some("sub"))
            .unwrap_err();
        assert!(err.contains("a is a link"), "{err}");
        let left: Vec<_> = fs::read_dir(root.path()).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_folder_holding_a_link_is_not_linked() {
        let outside = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join(file::FILE),
            MANIFEST.replace("[build]\ncommand = [\"sh\", \"build.sh\"]\n", ""),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), folder.path().join("dir")).unwrap();
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), "git", env());
        let err = installs.link(folder.path()).unwrap_err();
        assert!(err.contains("dir is a link"), "{err}");
        assert!(installs.inspect_link(folder.path()).is_err());
        assert!(installs.store().describe("test.marker").is_none());
    }

    #[test]
    fn a_linked_folder_that_gains_a_link_is_not_loaded_until_it_is_gone() {
        let outside = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join(file::FILE),
            MANIFEST.replace("[build]\ncommand = [\"sh\", \"build.sh\"]\n", ""),
        )
        .unwrap();
        fs::create_dir(folder.path().join("lib")).unwrap();
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), "git", env());
        installs.link(folder.path()).unwrap();
        assert!(installs.recheck_linked().is_empty());

        let lock = folder.path().join("lib/.#main.rs");
        std::os::unix::fs::symlink("me@host.1234:1700000000", &lock).unwrap();
        let planted = folder.path().join("lib/keys");
        std::os::unix::fs::symlink(outside.path(), &planted).unwrap();
        assert!(installs.recheck_linked().is_empty(), "seen once");
        assert!(installs.store().inspect("test.marker").is_ok());
        fs::remove_file(&planted).unwrap();
        assert!(
            installs.recheck_linked().is_empty(),
            "gone by the next check"
        );
        assert!(
            installs.recheck_linked().is_empty(),
            "an editor's lock is not packed"
        );

        std::os::unix::fs::symlink(outside.path(), &planted).unwrap();
        assert!(installs.recheck_linked().is_empty());
        assert_eq!(installs.recheck_linked(), ["test.marker"]);
        let err = installs.store().inspect("test.marker").unwrap_err();
        assert!(err.contains("lib/keys is a link"), "{err}");
        assert!(installs.recheck_linked().is_empty(), "marked once");

        fs::remove_file(&planted).unwrap();
        assert_eq!(installs.recheck_linked(), ["test.marker"]);
        assert!(installs.store().inspect("test.marker").is_ok());
    }

    #[test]
    fn an_id_differing_only_in_case_is_not_installed_over_another() {
        let manifest = MANIFEST.replace("[build]\ncommand = [\"sh\", \"build.sh\"]\n", "");
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join(file::FILE), &manifest).unwrap();
        fs::write(folder.path().join("hello.sh"), "mine").unwrap();
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), "git", env());
        let sheet = installs
            .inspect_git(&folder.path().display().to_string(), None)
            .unwrap();
        installs.confirm(&sheet.token, "test.marker").unwrap();

        let other = tempfile::tempdir().unwrap();
        fs::write(
            other.path().join(file::FILE),
            manifest.replace("test.marker", "Test.Marker"),
        )
        .unwrap();
        fs::write(other.path().join("hello.sh"), "theirs").unwrap();
        assert!(installs
            .inspect_git(&other.path().display().to_string(), None)
            .is_err());
        assert!(installs.link(other.path()).is_err());
        assert!(installs.remove("Test.Marker").is_err());
        let kept = installs.store().describe("test.marker").unwrap();
        assert_eq!(
            fs::read_to_string(kept.path.join("hello.sh")).unwrap(),
            "mine"
        );
    }

    #[cfg(unix)]
    #[test]
    fn writing_current_never_writes_through_a_planted_link() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let target = elsewhere.path().join("target");
        fs::write(&target, "untouched").unwrap();
        std::os::unix::fs::symlink(&target, home.path().join(format!("{CURRENT_FILE}.tmp")))
            .unwrap();
        write_current(home.path(), &json!({ "version": "1.0.0" })).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
        assert_eq!(
            read_current(home.path()).unwrap(),
            json!({ "version": "1.0.0" })
        );
        let left: Vec<String> = fs::read_dir(home.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 2, "{left:?}");
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
        // An id Vorn's own workflows start with is no extension's.
        for id in crate::pack::RESERVED_IDS {
            let reserved =
                format!("\"{id}\" is reserved: Vorn's own workflows' ids start with {id}:");
            let id_line = format!("id = \"{id}\"");
            assert_eq!(attempt("id = \"test.marker\"", &id_line), Err(reserved));
        }
        // Nor is one in the namespace of the extensions Vorn ships.
        let shipped = "\"vorn.connectors\" is reserved: ids starting with vorn. are the extensions Vorn ships";
        assert_eq!(
            attempt("id = \"test.marker\"", "id = \"vorn.connectors\""),
            Err(shipped.to_owned())
        );
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
    fn packs_a_manifest_naming_an_installed_agent() {
        let folder = tempfile::tempdir().unwrap();
        let manifest = |agent: &str| {
            format!(
                "id = \"test.agents\"\nname = \"Agents\"\nversion = \"1.0.0\"\napi = \"vorn:extension@1.0\"\npublisher = \"test\"\n\n[actions.hello]\ntitle = \"Hello\"\ncommand = [\"sh\", \"hello.sh\"]\n\n[activates]\nagent = [\"{agent}\"]\n"
            )
        };
        let write =
            |agent: &str| fs::write(folder.path().join(file::FILE), manifest(agent)).unwrap();
        write("acme-pack-test");
        assert!(super::pack(folder.path(), &env(), None, &[]).is_err());
        super::pack(folder.path(), &env(), None, &["acme-pack-test".to_owned()]).unwrap();
    }

    /// A `.vorn.tgz` of `files` under one `<top>/` folder, as vorn-pack writes one.
    fn tgz(top: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (path, bytes) in files {
            let mut header = tar::Header::new_ustar();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            tar.append_data(&mut header, format!("{top}/{path}"), *bytes)
                .unwrap();
        }
        let raw = tar.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &raw).unwrap();
        gz.finish().unwrap()
    }

    fn shipped_manifest(id: &str) -> String {
        format!(
            "id = \"{id}\"\nname = \"Shipped\"\nversion = \"1.2.0\"\napi = \"vorn:extension@1.0\"\npublisher = \"Vorn\"\n\n[actions.hello]\ntitle = \"Hello\"\ncommand = [\"sh\", \"hello.sh\"]\n"
        )
    }

    #[test]
    fn admits_vorns_namespace_only_for_the_pack_it_pinned() {
        let manifest = shipped_manifest("vorn.sample");
        let bytes = tgz(
            "vorn.sample-1.2.0",
            &[(file::FILE, manifest.as_bytes()), ("hello.sh", b"true\n")],
        );
        let root = tempfile::tempdir().unwrap();
        let pin = |sha256: String| Pinned {
            id: "vorn.sample".into(),
            version: "1.2.0".into(),
            sha256,
        };
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env())
            .trusting(Trust {
                pins: vec![pin(sha256_hex(&bytes))],
                ..Trust::default()
            });

        let unpinned = installs
            .inspect_archive(&bytes, json!({}), None)
            .unwrap_err();
        assert!(unpinned.contains("reserved"), "{unpinned}");
        let other = installs
            .inspect_archive(&bytes, json!({}), Some(&pin("00".repeat(32))))
            .unwrap_err();
        assert!(other.contains("not a pack this Vorn ships"), "{other}");
        let renamed = Pinned {
            id: "vorn.other".into(),
            ..pin(sha256_hex(&bytes))
        };
        assert!(installs
            .inspect_archive(&bytes, json!({}), Some(&renamed))
            .is_err());

        let sheet = installs
            .inspect_archive(
                &bytes,
                json!({"shipped": "1.2.0"}),
                Some(&pin(sha256_hex(&bytes))),
            )
            .unwrap();
        let pack = installs.confirm(&sheet.token, "vorn.sample").unwrap();
        assert_eq!(pack.version, "1.2.0");
        assert!(pack.path.join("hello.sh").is_file());
        let source = pack.source.unwrap();
        assert_eq!(source["shipped"], "1.2.0");
        assert_eq!(source["sha256"], sha256_hex(&bytes).as_str());
        let staging: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging"))
            .collect();
        assert!(staging.is_empty(), "{staging:?}");
    }

    #[test]
    fn a_pin_this_build_does_not_carry_never_admits_vorns_namespace() {
        let manifest = shipped_manifest("vorn.sample");
        let bytes = tgz(
            "vorn.sample-1.2.0",
            &[(file::FILE, manifest.as_bytes()), ("hello.sh", b"true\n")],
        );
        let root = tempfile::tempdir().unwrap();
        let shipped = Pinned {
            id: "vorn.sample".into(),
            version: "1.1.0".into(),
            sha256: "11".repeat(32),
        };
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env())
            .trusting(Trust {
                pins: vec![shipped],
                ..Trust::default()
            });
        // A pin as an unsigned catalog lists one: its own id, version and the bytes' checksum.
        let listed = Pinned {
            id: "vorn.sample".into(),
            version: "1.2.0".into(),
            sha256: sha256_hex(&bytes),
        };
        let refused = installs
            .inspect_archive(&bytes, json!({"catalog": "vorn.sample"}), Some(&listed))
            .unwrap_err();
        assert!(refused.contains("not a pack this Vorn ships"), "{refused}");
        assert!(installs.vouch(&listed).is_err());
        let theirs = Pinned {
            id: "acme.sample".into(),
            ..listed
        };
        assert!(
            installs.vouch(&theirs).is_ok(),
            "an id of its own needs no vouching"
        );
        assert!(installs.store().ids().is_empty());
    }

    #[test]
    fn refuses_the_id_of_a_bundled_extension() {
        let manifest = shipped_manifest("bundled.sample");
        let bytes = tgz(
            "bundled",
            &[(file::FILE, manifest.as_bytes()), ("hello.sh", b"true\n")],
        );
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env())
            .trusting(Trust {
                bundled: BTreeSet::from(["bundled.sample".to_owned()]),
                ..Trust::default()
            });
        let refused = installs
            .inspect_archive(&bytes, json!({}), None)
            .unwrap_err();
        assert!(refused.contains("Vorn bundles"), "{refused}");
        let listed = Pinned {
            id: "bundled.sample".into(),
            version: "1.2.0".into(),
            sha256: sha256_hex(&bytes),
        };
        let refused = installs
            .inspect_archive(&bytes, json!({}), Some(&listed))
            .unwrap_err();
        assert!(refused.contains("not a pack this Vorn ships"), "{refused}");

        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join(file::FILE), &manifest).unwrap();
        let refused = installs.inspect_link(folder.path()).unwrap_err();
        assert!(refused.contains("Vorn bundles"), "{refused}");

        let trusted = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env())
            .trusting(Trust {
                pins: vec![listed.clone()],
                bundled: BTreeSet::from(["bundled.sample".to_owned()]),
            });
        let sheet = trusted
            .inspect_archive(&bytes, json!({}), Some(&listed))
            .unwrap();
        trusted.confirm(&sheet.token, "bundled.sample").unwrap();
    }

    fn adapter_manifest(id: &str, agent: &str) -> String {
        format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi = \"vorn:extension@1.0\"\npublisher = \"test\"\n\n[agent]\nid = \"{agent}\"\nname = \"{agent}\"\n\n[agent.launch]\ncommand = \"{agent}\"\n"
        )
    }

    #[test]
    fn refuses_an_adapter_for_an_agent_another_runs_but_vorns_own() {
        let root = tempfile::tempdir().unwrap();
        let ours = adapter_manifest("vorn.agents.sample", "sample-agent");
        let ours = tgz("vorn", &[(file::FILE, ours.as_bytes())]);
        let pin = Pinned {
            id: "vorn.agents.sample".into(),
            version: "1.0.0".into(),
            sha256: sha256_hex(&ours),
        };
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env())
            .trusting(Trust {
                pins: vec![pin.clone()],
                ..Trust::default()
            });
        let install = |bytes: &[u8], pin: Option<&Pinned>, id: &str| {
            let sheet = installs.inspect_archive(bytes, json!({}), pin)?;
            installs.confirm(&sheet.token, id)
        };
        let theirs = |id: &str, agent: &str| {
            tgz("t", &[(file::FILE, adapter_manifest(id, agent).as_bytes())])
        };

        install(&theirs("acme.first", "sample-agent"), None, "acme.first").unwrap();
        let refused =
            install(&theirs("acme.second", "sample-agent"), None, "acme.second").unwrap_err();
        assert!(
            refused.contains("\"sample-agent\", which acme.first already runs"),
            "{refused}"
        );
        install(&theirs("acme.first", "sample-agent"), None, "acme.first")
            .expect("an update of the same adapter keeps its agent");
        install(&ours, Some(&pin), "vorn.agents.sample").expect("Vorn's own wins its agent id");
        install(&theirs("acme.other", "other-agent"), None, "acme.other").unwrap();
        assert_eq!(
            installs.store().ids(),
            ["acme.first", "acme.other", "vorn.agents.sample"]
        );
    }

    fn adapter_at(id: &str, agent: &str, version: &str) -> Vec<u8> {
        let manifest = adapter_manifest(id, agent)
            .replace("version = \"1.0.0\"", &format!("version = \"{version}\""));
        tgz("t", &[(file::FILE, manifest.as_bytes())])
    }

    #[test]
    fn a_link_records_the_agent_it_adapts() {
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env());
        let folder = tempfile::tempdir().unwrap();
        let manifest = adapter_manifest("acme.linked", "sample-agent");
        fs::write(folder.path().join(file::FILE), manifest).unwrap();
        let pack = installs.link(folder.path()).unwrap();
        assert_eq!(pack.linked_agent.as_deref(), Some("sample-agent"));
        assert!(pack.installed_at.as_u64().is_some());
    }

    #[test]
    fn an_agent_taken_after_the_sheet_is_refused_at_confirm() {
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env());
        let late = installs
            .inspect_archive(
                &adapter_at("acme.late", "sample-agent", "1.0.0"),
                json!({}),
                None,
            )
            .unwrap();
        let first = installs
            .inspect_archive(
                &adapter_at("acme.first", "sample-agent", "1.0.0"),
                json!({}),
                None,
            )
            .unwrap();
        installs.confirm(&first.token, "acme.first").unwrap();
        let refused = installs.confirm(&late.token, "acme.late").unwrap_err();
        assert!(
            refused.contains("which acme.first already runs"),
            "{refused}"
        );
        assert_eq!(installs.store().ids(), ["acme.first"]);
        let left: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_build_that_rewrites_its_manifest_installs_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let manifest = adapter_manifest("acme.builder", "own-agent").replace(
            "[agent]",
            "[build]\ncommand = [\"sh\", \"build.sh\"]\n\n[agent]",
        );
        fs::write(folder.path().join(file::FILE), manifest).unwrap();
        fs::write(
            folder.path().join("build.sh"),
            "sed 's/own-agent/claude/' vorn-extension.toml > m && mv m vorn-extension.toml\n",
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env());
        let sheet = installs
            .inspect_git(&folder.path().display().to_string(), None)
            .unwrap();
        let refused = installs.confirm(&sheet.token, "acme.builder").unwrap_err();
        assert!(refused.contains("build changed its manifest"), "{refused}");
        assert!(installs.store().ids().is_empty());
    }

    #[test]
    fn a_rollback_to_an_agent_another_took_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env());
        let install = |bytes: &[u8], id: &str| {
            let sheet = installs.inspect_archive(bytes, json!({}), None)?;
            installs.confirm(&sheet.token, id)
        };
        install(&adapter_at("acme.a", "alpha", "1.0.0"), "acme.a").unwrap();
        install(&adapter_at("acme.a", "beta", "2.0.0"), "acme.a").unwrap();
        install(&adapter_at("acme.b", "alpha", "1.0.0"), "acme.b").unwrap();
        let refused = installs.rollback("acme.a").unwrap_err();
        assert!(refused.contains("which acme.b already runs"), "{refused}");
        assert_eq!(
            installs.store().describe("acme.a").unwrap().version,
            "2.0.0"
        );

        installs.remove("acme.b").unwrap();
        let back = installs.rollback("acme.a").unwrap();
        assert_eq!(back.version, "1.0.0");
    }

    #[test]
    fn an_update_and_a_rollback_keep_when_it_was_first_installed() {
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env());
        let install = |bytes: &[u8]| {
            let sheet = installs.inspect_archive(bytes, json!({}), None).unwrap();
            installs.confirm(&sheet.token, "acme.a").unwrap()
        };
        let first = install(&adapter_at("acme.a", "alpha", "1.0.0")).installed_at;
        assert!(first.as_u64().is_some(), "{first}");
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_eq!(
            install(&adapter_at("acme.a", "alpha", "2.0.0")).installed_at,
            first
        );
        assert_eq!(installs.rollback("acme.a").unwrap().installed_at, first);
    }

    #[test]
    fn a_rollback_of_vorns_own_goes_only_to_a_version_this_build_pins() {
        let root = tempfile::tempdir().unwrap();
        let pack = |version: &str| {
            let manifest = shipped_manifest("vorn.sample").replace("1.2.0", version);
            let bytes = tgz("p", &[(file::FILE, manifest.as_bytes())]);
            let pin = Pinned {
                id: "vorn.sample".into(),
                version: version.into(),
                sha256: sha256_hex(&bytes),
            };
            (bytes, pin)
        };
        let ((old, old_pin), (new, new_pin)) = (pack("1.0.0"), pack("1.1.0"));
        let trusting = |pins: Vec<Pinned>| {
            Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env()).trusting(
                Trust {
                    pins,
                    ..Trust::default()
                },
            )
        };
        let both = trusting(vec![old_pin.clone(), new_pin.clone()]);
        for (bytes, pin) in [(&old, &old_pin), (&new, &new_pin)] {
            let sheet = both
                .inspect_archive(bytes, json!({ "shipped": pin.version }), Some(pin))
                .unwrap();
            both.confirm(&sheet.token, "vorn.sample").unwrap();
        }

        let refused = trusting(vec![new_pin.clone()])
            .rollback("vorn.sample")
            .unwrap_err();
        assert!(refused.contains("not a pack this Vorn ships"), "{refused}");
        assert_eq!(
            both.store().describe("vorn.sample").unwrap().version,
            "1.1.0"
        );

        let back = both.rollback("vorn.sample").unwrap();
        assert_eq!(back.version, "1.0.0");
        assert_eq!(back.source.unwrap()["sha256"], old_pin.sha256.as_str());
        let forward = both.rollback("vorn.sample").unwrap();
        assert_eq!(forward.source.unwrap()["sha256"], new_pin.sha256.as_str());
    }

    #[test]
    fn refuses_a_pack_whose_pax_size_is_over_the_cap() {
        let manifest = shipped_manifest("acme.sample");
        let mut tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_ustar();
        header.set_size(manifest.len() as u64);
        header.set_mode(0o644);
        tar.append_data(
            &mut header,
            format!("a/{}", file::FILE),
            manifest.as_bytes(),
        )
        .unwrap();
        let big = ARCHIVE_MAX_BYTES + 1;
        tar.append_pax_extensions([("size", big.to_string().as_bytes())])
            .unwrap();
        // The ustar size says nothing; the PAX record before it gives the real size.
        let mut header = tar::Header::new_ustar();
        header.set_size(0);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_path("a/big").unwrap();
        header.set_cksum();
        tar.append(&header, std::io::repeat(0).take(big)).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar.into_inner().unwrap()).unwrap();
        let bytes = gz.finish().unwrap();

        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env());
        let refused = installs
            .inspect_archive(&bytes, json!({}), None)
            .unwrap_err();
        assert!(refused.contains("larger than a pack may be"), "{refused}");
        let left: Vec<_> = fs::read_dir(root.path()).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_git_install_claiming_vorns_namespace_is_refused() {
        let Some(git) = git() else { return };
        let source = repo(&[
            (file::FILE, &shipped_manifest("vorn.sample")),
            ("hello.sh", "true\n"),
        ]);
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), git, env());
        let url = format!("file://{}", source.path().display());
        let refused = installs.inspect_git(&url, None).unwrap_err();
        assert!(refused.contains("reserved"), "{refused}");
        assert!(installs.store().describe("vorn.sample").is_none());
    }

    #[test]
    fn an_unpinned_pack_installs_under_an_id_of_its_own() {
        let manifest = shipped_manifest("acme.sample");
        let bytes = tgz(
            "acme",
            &[(file::FILE, manifest.as_bytes()), ("hello.sh", b"true\n")],
        );
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env());
        let sheet = installs.inspect_archive(&bytes, json!({}), None).unwrap();
        installs.confirm(&sheet.token, "acme.sample").unwrap();
        assert!(installs.store().describe("acme.sample").is_some());
    }

    #[test]
    fn refuses_a_pack_that_reaches_outside_its_folder() {
        let manifest = shipped_manifest("acme.sample");
        let root = tempfile::tempdir().unwrap();
        let installs = Installs::new(PackStore::new(root.path()), PathBuf::from("git"), env());
        let two = tgz("a", &[(file::FILE, manifest.as_bytes())]);
        let mut both = tar::Builder::new(Vec::new());
        for top in ["a", "b"] {
            let mut header = tar::Header::new_ustar();
            header.set_size(manifest.len() as u64);
            header.set_entry_type(tar::EntryType::Regular);
            both.append_data(
                &mut header,
                format!("{top}/{}", file::FILE),
                manifest.as_bytes(),
            )
            .unwrap();
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &both.into_inner().unwrap()).unwrap();
        let refused = installs
            .inspect_archive(&gz.finish().unwrap(), json!({}), None)
            .unwrap_err();
        assert!(refused.contains("more than one folder"), "{refused}");

        let mut link = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        link.append_link(&mut header, "a/evil", "/etc/passwd")
            .unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &link.into_inner().unwrap()).unwrap();
        let refused = installs
            .inspect_archive(&gz.finish().unwrap(), json!({}), None)
            .unwrap_err();
        assert!(refused.contains("not a plain file"), "{refused}");

        assert!(installs
            .inspect_archive(b"not a tgz", json!({}), None)
            .is_err());
        assert!(installs.inspect_archive(&two, json!({}), None).is_ok());
        let staged = fs::read_dir(root.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging"))
            .count();
        assert_eq!(staged, 1, "only the pack that read stays staged");
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

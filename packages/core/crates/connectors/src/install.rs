//! Installing, rolling back and removing packs: an archive is fetched or read,
//! unpacked into a staging directory and verified there, and only a rename
//! puts it in place. What an inspection verified is held under a token, so
//! confirming installs the bytes that were checked.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::manifest::{self, Kind, Manifest};
use crate::pack::{is_safe_id, InstalledPack, PackStore, ENTRY_FILE};
use crate::fetch::Fetch;
use crate::sdk::outdated_message;

/// Largest archive Vorn will install, matched by the SDK's own pack gate.
pub const MAX_PACK_BYTES: u64 = 8 * 1024 * 1024;
/// A small archive can still unpack to something enormous.
pub const MAX_UNPACKED_BYTES: u64 = 32 * 1024 * 1024;
/// A slow mirror must not wedge an install forever.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// Long enough to read a confirm sheet, short enough that abandoning one costs nothing.
const STAGED_TTL: Duration = Duration::from_secs(10 * 60);

const MANIFEST_FILE: &str = "manifest.json";
const CURRENT_FILE: &str = "current.json";
/// Everything a pack may carry besides an extension's pages.
const PACK_FILES: [&str; 3] = [MANIFEST_FILE, ENTRY_FILE, "package.json"];
/// Where an extension's pane pages live.
const WEB_PREFIX: &str = "web/";
/// What a page may be made of.
pub const WEB_FILE_TYPES: [&str; 15] = [
    "html", "css", "js", "mjs", "json", "svg", "png", "jpg", "jpeg", "webp", "gif", "woff",
    "woff2", "txt", "md",
];
/// The connector ids the app answers to itself.
const RESERVED_IDS: [&str; 3] = ["mcp", "http", "sdk"];

/// Where a pack comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    Url { url: String, sha256: Option<String> },
    /// Verified by an inspection, held under this token.
    Staged(String),
}

impl Source {
    /// Reads a source a client sent, or says why it cannot be used, in the
    /// server's words (`unusableSource`).
    pub fn parse(raw: &Value) -> Result<Source, String> {
        let Some(source) = raw.as_object() else {
            return Err("That is not a pack to install".into());
        };
        let text = |k: &str| source.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
        match source.get("kind").and_then(Value::as_str) {
            Some("file") => text("path")
                .map(|p| Source::File(PathBuf::from(p)))
                .ok_or_else(|| "That file path is empty".into()),
            Some("staged") => text("token")
                .map(|t| Source::Staged(t.to_owned()))
                .ok_or_else(|| "That pack has expired".into()),
            Some("url") => {
                let url = source.get("url").and_then(Value::as_str).unwrap_or("");
                usable_url(url)?;
                Ok(Source::Url {
                    url: url.to_owned(),
                    sha256: text("sha256").map(str::to_owned),
                })
            }
            _ => Err("That is not a pack to install".into()),
        }
    }

    fn describe(&self) -> String {
        match self {
            Source::File(p) => p.display().to_string(),
            Source::Url { url, .. } => url.clone(),
            Source::Staged(_) => "a checked pack".into(),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Source::File(_) => "file",
            Source::Url { .. } => "url",
            Source::Staged(_) => "staged",
        }
    }
}

/// A pack is fetched over https, or plain http from this machine only.
fn usable_url(url: &str) -> Result<(), String> {
    let refused = || "That is not a URL a pack can be fetched from".to_owned();
    let (scheme, rest) = url.split_once("://").ok_or_else(refused)?;
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_port
        .rsplit_once('@')
        .map_or(host_port, |(_, h)| h)
        .split(':')
        .next()
        .unwrap_or("");
    if host.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
        return Err(refused());
    }
    match scheme.to_ascii_lowercase().as_str() {
        "https" => Ok(()),
        "http" if matches!(host, "localhost" | "127.0.0.1") => Ok(()),
        _ => Err("A pack is fetched over https, or from this machine".into()),
    }
}

/// How an install is going, for the clients watching it.
pub type Progress<'a> = &'a (dyn Fn(Value) + Send + Sync);


struct Staged {
    staging: PathBuf,
    contents: PathBuf,
    manifest: Manifest,
    expires: Instant,
}

/// Installs into a pack store, and holds what inspections verified.
pub struct Installer {
    store: PackStore,
    staged: Mutex<HashMap<String, Staged>>,
}

impl std::fmt::Debug for Installer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Installer")
            .field("root", &self.store.root())
            .finish_non_exhaustive()
    }
}

impl Installer {
    pub fn new(store: PackStore) -> Installer {
        Installer {
            store,
            staged: Mutex::default(),
        }
    }

    pub fn store(&self) -> &PackStore {
        &self.store
    }

    fn staged(&self) -> std::sync::MutexGuard<'_, HashMap<String, Staged>> {
        self.staged.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sweep(&self) {
        let now = Instant::now();
        self.staged().retain(|_, s| {
            let keep = s.expires > now;
            if !keep {
                let _ = fs::remove_dir_all(&s.staging);
            }
            keep
        });
    }

    fn staging_dir(&self) -> Result<PathBuf, String> {
        let root = self.store.root();
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        Ok(root.join(format!(".tmp-{}", token())))
    }

    /// Verifies a pack and describes it without installing it
    /// (`ConnectorPackPreview`).
    pub fn inspect(&self, source: &Source, download: &dyn Fetch) -> Value {
        self.sweep();
        let staging = match self.staging_dir() {
            Ok(dir) => dir,
            Err(error) => return json!({ "ok": false, "error": error }),
        };
        match stage(source, download, &staging, &|_| {}) {
            Ok((contents, manifest)) => {
                let installed = self.store.describe(&manifest.id);
                let mut preview = Map::new();
                preview.insert("id".into(), json!(manifest.id));
                preview.insert("name".into(), json!(manifest.name));
                preview.insert("version".into(), json!(manifest.version));
                if let Some(d) = &manifest.description {
                    preview.insert("description".into(), json!(d));
                }
                if let Some(icon) = &manifest.icon {
                    preview.insert("icon".into(), json!(icon));
                }
                if let Some(auth) = &manifest.auth {
                    preview.insert("auth".into(), json!(auth));
                }
                extension_facts(&manifest, &mut preview);
                preview.insert("triggers".into(), json!(manifest.triggers));
                preview.insert("actions".into(), json!(manifest.actions));
                preview.insert("env".into(), json!(manifest.env));
                if let Some(installed) = installed {
                    preview.insert("installedVersion".into(), json!(installed.version));
                }
                let token = token();
                preview.insert("token".into(), json!(token));
                self.staged().insert(
                    token,
                    Staged {
                        staging,
                        contents,
                        manifest,
                        expires: Instant::now() + STAGED_TTL,
                    },
                );
                json!({ "ok": true, "preview": preview })
            }
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                warn!("[packs] inspect of {} refused: {error}", source.describe());
                json!({ "ok": false, "error": error })
            }
        }
    }

    /// Installs a pack (`ConnectorPackResult`); the rename is the commit point.
    pub fn install(&self, source: &Source, download: &dyn Fetch, progress: Progress) -> Value {
        let held = match source {
            Source::Staged(token) => match self.staged().remove(token) {
                Some(held) => Some(held),
                None => {
                    return json!({ "ok": false, "error": "That pack was checked too long ago; open it again to install it" })
                }
            },
            _ => None,
        };
        let id = std::cell::RefCell::new(held.as_ref().map(|h| h.manifest.id.clone()));
        let report = |mut event: Value| {
            if let Some(id) = id.borrow().as_ref() {
                event["id"] = json!(id);
                progress(event);
            }
        };
        let staging = match &held {
            Some(h) => h.staging.clone(),
            None => match self.staging_dir() {
                Ok(dir) => dir,
                Err(error) => return json!({ "ok": false, "error": error }),
            },
        };
        let result = (|| -> Result<InstalledPack, String> {
            let (contents, manifest) = match held {
                Some(h) => (h.contents, h.manifest),
                None => stage(source, download, &staging, &report)?,
            };
            *id.borrow_mut() = Some(manifest.id.clone());
            report(json!({ "phase": "installing", "version": manifest.version }));
            self.commit(&contents, &manifest)?;
            let pack = self
                .store
                .describe(&manifest.id)
                .ok_or("The pack was installed but could not be read back")?;
            info!(
                "[packs] installed {}@{} from {}",
                manifest.id,
                manifest.version,
                source.kind()
            );
            report(json!({ "phase": "installed", "version": manifest.version }));
            Ok(pack)
        })();
        let _ = fs::remove_dir_all(&staging);
        match result {
            Ok(pack) => json!({ "ok": true, "pack": pack }),
            Err(error) => {
                warn!("[packs] install from {} refused: {error}", source.describe());
                report(json!({ "phase": "failed", "error": error }));
                json!({ "ok": false, "error": error })
            }
        }
    }

    fn commit(&self, contents: &Path, manifest: &Manifest) -> Result<(), String> {
        let dir = self.store.root().join(&manifest.id);
        let current = read_current(&dir);
        let target = dir.join(&manifest.version);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let displaced = target
            .exists()
            .then(|| dir.join(format!("{}.replaced-{}", manifest.version, token())));
        if let Some(aside) = &displaced {
            fs::rename(&target, aside).map_err(|e| e.to_string())?;
        }
        if let Err(err) = fs::rename(contents, &target) {
            if let Some(aside) = &displaced {
                let _ = fs::rename(aside, &target);
            }
            return Err(err.to_string());
        }
        if let Some(aside) = &displaced {
            let _ = fs::remove_dir_all(aside);
        }
        let previous = match &current {
            Some(c) if c.version != manifest.version => Some(c.version.clone()),
            Some(c) => c.previous.clone(),
            None => None,
        };
        write_current(&dir, &manifest.version, previous.as_deref())?;
        prune(&dir, &[Some(manifest.version.as_str()), previous.as_deref()]);
        Ok(())
    }

    /// Swaps back to the one version kept behind the current one.
    pub fn rollback(&self, id: &str) -> Value {
        if !is_safe_id(id) {
            return json!({ "ok": false, "error": format!("\"{id}\" is not a usable connector id") });
        }
        let dir = self.store.root().join(id);
        let Some(previous) = read_current(&dir).and_then(|c| c.previous.map(|p| (c.version, p)))
        else {
            return json!({ "ok": false, "error": "There is no earlier version to roll back to" });
        };
        let (version, previous) = previous;
        if !dir.join(&previous).join(ENTRY_FILE).exists() {
            return json!({ "ok": false, "error": format!("Version {previous} is no longer on disk") });
        }
        if let Err(error) = write_current(&dir, &previous, Some(&version)) {
            return json!({ "ok": false, "error": error });
        }
        match self.store.describe(id) {
            Some(pack) => {
                info!("[packs] rolled {id} back to {}", pack.version);
                json!({ "ok": true, "pack": pack })
            }
            None => json!({ "ok": false, "error": "The earlier version could not be read back" }),
        }
    }

    /// Removes every version of a pack.
    pub fn remove(&self, id: &str) -> Value {
        if !is_safe_id(id) {
            return json!({ "ok": false, "error": format!("\"{id}\" is not a usable connector id") });
        }
        let dir = self.store.root().join(id);
        if !dir.exists() {
            return json!({ "ok": false, "error": format!("{id} is not installed") });
        }
        if let Err(err) = fs::remove_dir_all(&dir) {
            return json!({ "ok": false, "error": err.to_string() });
        }
        info!("[packs] removed {id}");
        json!({ "ok": true })
    }
}

/// What an extension adds and asks for, on every description of a pack.
fn extension_facts(manifest: &Manifest, out: &mut Map<String, Value>) {
    if manifest.kind != Kind::Extension {
        return;
    }
    out.insert("kind".into(), json!("extension"));
    if let Some(c) = &manifest.contributes {
        out.insert("contributes".into(), json!(c));
    }
    if let Some(p) = &manifest.permissions {
        out.insert("permissions".into(), json!(p));
    }
    if let Some(a) = &manifest.activates {
        out.insert("activates".into(), json!(a));
    }
}

struct Current {
    version: String,
    previous: Option<String>,
}

fn read_current(dir: &Path) -> Option<Current> {
    let text = fs::read_to_string(dir.join(CURRENT_FILE)).ok()?;
    let parsed: Value = serde_json::from_str(&text).ok()?;
    let version = parsed
        .get("version")?
        .as_str()
        .filter(|v| is_safe_version(v))?;
    Some(Current {
        version: version.to_owned(),
        previous: parsed
            .get("previousVersion")
            .and_then(Value::as_str)
            .filter(|v| is_safe_version(v))
            .map(str::to_owned),
    })
}

fn write_current(dir: &Path, version: &str, previous: Option<&str>) -> Result<(), String> {
    let mut current = Map::new();
    current.insert("version".into(), json!(version));
    if let Some(previous) = previous {
        current.insert("previousVersion".into(), json!(previous));
    }
    current.insert("installedAt".into(), json!(now_ms()));
    let text = serde_json::to_string_pretty(&Value::Object(current)).map_err(|e| e.to_string())?;
    fs::write(dir.join(CURRENT_FILE), text).map_err(|e| e.to_string())
}

fn prune(dir: &Path, keep: &[Option<&str>]) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|t| t.is_dir()) && !keep.contains(&Some(name.as_str())) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn is_safe_version(version: &str) -> bool {
    let b = version.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-'))
        && !version.contains("..")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A name nobody can guess, for a staging directory or an inspection.
fn token() -> String {
    let mut bytes = [0u8; 12];
    if getrandom::fill(&mut bytes).is_err() {
        bytes[..8].copy_from_slice(&now_ms().to_le_bytes());
    }
    data_encoding::HEXLOWER.encode(&bytes)
}

fn size_message(bytes: u64) -> String {
    format!(
        "The pack is {} KB; Vorn installs at most {} MB",
        (bytes as f64 / 1024.0).round(),
        MAX_PACK_BYTES / 1024 / 1024
    )
}

fn unpacked_message(bytes: u64) -> String {
    format!(
        "The pack unpacks to {} MB; Vorn installs at most {} MB",
        (bytes as f64 / 1024.0 / 1024.0).round(),
        MAX_UNPACKED_BYTES / 1024 / 1024
    )
}

/// The archive's bytes, from a file or a download.
fn read_source(
    source: &Source,
    download: &dyn Fetch,
    report: &dyn Fn(Value),
) -> Result<Vec<u8>, String> {
    match source {
        Source::Staged(_) => Err("That pack was already checked".into()),
        Source::File(path) => {
            let size = fs::metadata(path).map_err(|e| e.to_string())?.len();
            if size > MAX_PACK_BYTES {
                return Err(size_message(size));
            }
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            report(json!({ "phase": "downloading", "percent": 100 }));
            Ok(bytes)
        }
        Source::Url { url, sha256 } => {
            let last = std::cell::Cell::new(-1i64);
            let bytes = download.get(url, DOWNLOAD_TIMEOUT, MAX_PACK_BYTES, &|received, total| {
                if total == 0 {
                    return;
                }
                let percent = ((received as f64 / total as f64) * 100.0).round() as i64;
                if percent != last.get() {
                    last.set(percent);
                    report(json!({ "phase": "downloading", "percent": percent }));
                }
            })?;
            if let Some(expected) = sha256 {
                let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(&bytes));
                if digest != expected.to_lowercase() {
                    return Err(
                        "The downloaded pack does not match the checksum the catalog published"
                            .into(),
                    );
                }
            }
            Ok(bytes)
        }
    }
}

/// Fetches, unpacks and verifies into `staging`: the pack's root and manifest.
fn stage(
    source: &Source,
    download: &dyn Fetch,
    staging: &Path,
    report: &dyn Fn(Value),
) -> Result<(PathBuf, Manifest), String> {
    let archive = read_source(source, download, report)?;
    if archive.len() as u64 > MAX_PACK_BYTES {
        return Err(size_message(archive.len() as u64));
    }
    report(json!({ "phase": "verifying" }));
    let unpacked = staging.join("unpacked");
    fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    unpack(&archive, &unpacked)?;
    let contents = pack_root(&unpacked);
    let manifest = verify_dir(&contents)?;
    if !is_safe_id(&manifest.id) {
        return Err(format!("\"{}\" is not a usable connector id", manifest.id));
    }
    if RESERVED_IDS.contains(&manifest.id.to_lowercase().as_str()) {
        return Err(format!(
            "\"{}\" is the id of a connector Vorn already ships",
            manifest.id
        ));
    }
    if !is_safe_version(&manifest.version) {
        return Err(format!(
            "\"{}\" is not a usable connector version",
            manifest.version
        ));
    }
    Ok((contents, manifest))
}

/// Whether an archive entry may be written: a file or a directory, inside the root.
pub fn is_safe_entry(path: &str, file_or_dir: bool) -> bool {
    if !file_or_dir || path.starts_with('/') {
        return false;
    }
    let b = path.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return false;
    }
    !path.split(['/', '\\']).any(|seg| seg == "..")
}

/// Unpacks a gzipped tarball, refusing what could write outside `dir` and
/// stopping as soon as the entries claim more than [`MAX_UNPACKED_BYTES`].
fn unpack(archive: &[u8], dir: &Path) -> Result<(), String> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut total = 0u64;
    let entries = tar.entries().map_err(|e| format!("The pack is not a readable archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("The pack is not a readable archive: {e}"))?;
        let kind = entry.header().entry_type();
        let path = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        if !is_safe_entry(&path, kind.is_file() || kind.is_dir()) {
            continue;
        }
        total += entry.header().size().unwrap_or(0);
        if total > MAX_UNPACKED_BYTES {
            return Err(unpacked_message(total));
        }
        if kind.is_dir() {
            fs::create_dir_all(dir.join(&path)).map_err(|e| e.to_string())?;
            continue;
        }
        let target = dir.join(&path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(MAX_UNPACKED_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        fs::write(&target, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A tarball may wrap everything in one directory; a `.vorn.tgz` does not.
fn pack_root(dir: &Path) -> PathBuf {
    if dir.join(MANIFEST_FILE).exists() {
        return dir.to_owned();
    }
    let entries: Vec<fs::DirEntry> = fs::read_dir(dir)
        .map(|e| e.filter_map(Result::ok).collect())
        .unwrap_or_default();
    match entries.as_slice() {
        [only] if only.file_type().is_ok_and(|t| t.is_dir()) => only.path(),
        _ => dir.to_owned(),
    }
}

/// Every file under `dir`, as `/`-separated paths relative to it.
fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> std::io::Result<()> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
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

fn directory_bytes(dir: &Path) -> u64 {
    let mut files = Vec::new();
    if walk(dir, "", &mut files).is_err() {
        return 0;
    }
    files
        .iter()
        .filter_map(|f| fs::metadata(dir.join(f)).ok())
        .map(|m| m.len())
        .sum()
}

/// The directories this manifest's own pages sit in, each with its trailing slash.
fn web_directories(manifest: &Manifest) -> Vec<String> {
    let Some(panes) = manifest
        .contributes
        .as_ref()
        .filter(|_| manifest.kind == Kind::Extension)
        .and_then(|c| serde_json::to_value(c).ok())
        .and_then(|c| c.get("panes").cloned())
    else {
        return Vec::new();
    };
    let mut dirs: Vec<String> = Vec::new();
    for pane in panes.as_array().into_iter().flatten() {
        let Some(web) = pane.get("web").and_then(Value::as_str) else {
            continue;
        };
        if !web.starts_with(WEB_PREFIX) {
            continue;
        }
        let dir = format!("{}/", &web[..web.rfind('/').unwrap_or(0)]);
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

fn serves_a_page(file: &str, dirs: &[String]) -> bool {
    if !dirs.iter().any(|d| file.starts_with(d.as_str())) {
        return false;
    }
    file.rsplit_once('.')
        .is_some_and(|(_, ext)| WEB_FILE_TYPES.contains(&ext.to_lowercase().as_str()))
}

/// Refuses anything needing an install step, running code, or unpacking too
/// large; the manifest when the pack is one Vorn installs (`verifyPackDir`).
pub fn verify_dir(dir: &Path) -> Result<Manifest, String> {
    let mut files = Vec::new();
    walk(dir, "", &mut files).map_err(|e| e.to_string())?;
    if !files.iter().any(|f| f == MANIFEST_FILE) {
        return Err("The pack has no manifest.json".into());
    }
    if !files.iter().any(|f| f == ENTRY_FILE) {
        return Err("The pack has no entry to run".into());
    }
    for file in files.iter().filter(|f| f.ends_with("package.json")) {
        let Ok(pkg) = fs::read_to_string(dir.join(file))
            .map_err(|_| ())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|_| ()))
        else {
            continue;
        };
        let non_empty = |k: &str| pkg.get(k).and_then(Value::as_object).is_some_and(|m| !m.is_empty());
        if non_empty("dependencies") {
            return Err(format!("{file} declares dependencies; a pack must carry everything it needs so it can launch with no registry"));
        }
        if non_empty("scripts") {
            return Err(format!(
                "{file} declares scripts; a pack is installed by copying files, never by running them"
            ));
        }
    }
    let payload = fs::read_to_string(dir.join(MANIFEST_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .ok_or("The pack has no readable manifest.json")?;
    let manifest = manifest::read(&payload).map_err(|e| e.to_string())?;
    if manifest.protocol.is_none() {
        return Err(outdated_message(&manifest.name));
    }
    let dirs = web_directories(&manifest);
    let strays: Vec<&str> = files
        .iter()
        .map(String::as_str)
        .filter(|f| !PACK_FILES.contains(f) && !serves_a_page(f, &dirs))
        .collect();
    if !strays.is_empty() {
        return Err(format!(
            "The pack carries {}; a pack is {MANIFEST_FILE}, {ENTRY_FILE} and the pages an extension's own panes name under {WEB_PREFIX}",
            strays.join(", ")
        ));
    }
    let bytes = directory_bytes(dir);
    if bytes > MAX_UNPACKED_BYTES {
        return Err(unpacked_message(bytes));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoNetwork;
    impl Fetch for NoNetwork {
        fn get(&self, _: &str, _: Duration, _: u64, _: &dyn Fn(u64, u64)) -> Result<Vec<u8>, String> {
            Err("offline".into())
        }
    }

    /// Bytes served from memory, as a download would serve them.
    struct Served(Vec<u8>);
    impl Fetch for Served {
        fn get(&self, _: &str, _: Duration, _: u64, progress: &dyn Fn(u64, u64)) -> Result<Vec<u8>, String> {
            let total = self.0.len() as u64;
            progress(total / 2, total);
            progress(total, total);
            Ok(self.0.clone())
        }
    }

    fn connector(version: &str) -> Value {
        json!({ "id": "tickets", "name": "Tickets", "version": version, "protocol": 1,
                "actions": [{ "type": "create", "label": "Create" }] })
    }

    /// A gzipped tarball of `files`, wrapped in `package/` as npm packs it.
    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("package/{path}"), *bytes)
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn pack_file(dir: &Path, version: &str) -> PathBuf {
        let manifest = connector(version).to_string();
        let path = dir.join(format!("tickets-{version}.tgz"));
        fs::write(
            &path,
            archive(&[
                ("manifest.json", manifest.as_bytes()),
                ("index.js", b"process.stdin.resume()\n"),
            ]),
        )
        .unwrap();
        path
    }

    #[test]
    fn inspects_installs_updates_and_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let installer = Installer::new(PackStore::new(dir.path().join("connectors")));
        let first = pack_file(dir.path(), "1.0.0");
        let preview = installer.inspect(&Source::File(first), &NoNetwork);
        assert_eq!(preview["ok"], true, "{preview}");
        assert_eq!(preview["preview"]["id"], "tickets");
        assert!(preview["preview"].get("installedVersion").is_none());
        let token = preview["preview"]["token"].as_str().unwrap().to_owned();

        let heard = Mutex::new(Vec::new());
        let progress = |v: Value| heard.lock().unwrap().push(v["phase"].as_str().unwrap().to_owned());
        let installed = installer.install(&Source::Staged(token.clone()), &NoNetwork, &progress);
        assert_eq!(installed["ok"], true, "{installed}");
        assert_eq!(installed["pack"]["version"], "1.0.0");
        assert_eq!(*heard.lock().unwrap(), ["installing", "installed"]);
        // The token is spent.
        let again = installer.install(&Source::Staged(token), &NoNetwork, &|_| {});
        assert_eq!(again["ok"], false);

        let second = pack_file(dir.path(), "2.0.0");
        let bytes = fs::read(&second).unwrap();
        let sha = data_encoding::HEXLOWER.encode(&Sha256::digest(&bytes));
        let url = Source::Url { url: "https://example.com/t.tgz".into(), sha256: Some(sha) };
        let updated = installer.install(&url, &Served(bytes.clone()), &|_| {});
        assert_eq!(updated["pack"]["version"], "2.0.0", "{updated}");
        assert_eq!(updated["pack"]["previousVersion"], "1.0.0");

        let back = installer.rollback("tickets");
        assert_eq!(back["pack"]["version"], "1.0.0", "{back}");
        assert_eq!(back["pack"]["previousVersion"], "2.0.0");

        let wrong = Source::Url { url: "https://example.com/t.tgz".into(), sha256: Some("00".into()) };
        let refused = installer.install(&wrong, &Served(bytes), &|_| {});
        assert!(refused["error"].as_str().unwrap().contains("checksum"));

        assert_eq!(installer.remove("tickets"), json!({ "ok": true }));
        assert_eq!(installer.remove("tickets")["error"], "tickets is not installed");
        assert_eq!(installer.rollback("tickets")["error"], "There is no earlier version to roll back to");
    }

    #[test]
    fn refuses_what_a_pack_may_not_carry() {
        let dir = tempfile::tempdir().unwrap();
        let installer = Installer::new(PackStore::new(dir.path().join("c")));
        let manifest = connector("1.0.0").to_string();
        let cases: Vec<(Vec<(&str, &[u8])>, &str)> = vec![
            (vec![("index.js", b"x")], "The pack has no manifest.json"),
            (vec![("manifest.json", manifest.as_bytes())], "The pack has no entry to run"),
            (
                vec![("manifest.json", manifest.as_bytes()), ("index.js", b"x"), ("native.node", b"x")],
                "The pack carries native.node",
            ),
            (
                vec![
                    ("manifest.json", manifest.as_bytes()),
                    ("index.js", b"x"),
                    ("package.json", br#"{"dependencies":{"a":"1"}}"#),
                ],
                "package.json declares dependencies",
            ),
            (
                vec![
                    ("manifest.json", br#"{"id":"mcp","name":"M","version":"1","protocol":1,"actions":[{"type":"a"}]}"#),
                    ("index.js", b"x"),
                ],
                "the id of a connector Vorn already ships",
            ),
            (
                vec![
                    ("manifest.json", br#"{"id":"old","name":"Old","version":"1","actions":[{"type":"a"}]}"#),
                    ("index.js", b"x"),
                ],
                "Old was built for an older Vorn",
            ),
        ];
        for (files, wanted) in cases {
            let path = dir.path().join("p.tgz");
            fs::write(&path, archive(&files)).unwrap();
            let answer = installer.inspect(&Source::File(path), &NoNetwork);
            let error = answer["error"].as_str().unwrap_or_default();
            assert!(error.contains(wanted), "{error} should say {wanted}");
        }
        // Nothing is left staged by a refusal.
        let left: Vec<_> = fs::read_dir(dir.path().join("c")).unwrap().collect();
        assert!(left.is_empty());
    }

    #[test]
    fn reads_sources_as_the_server_did() {
        assert_eq!(Source::parse(&json!(null)).unwrap_err(), "That is not a pack to install");
        assert_eq!(Source::parse(&json!({ "kind": "file", "path": "" })).unwrap_err(), "That file path is empty");
        assert_eq!(Source::parse(&json!({ "kind": "staged" })).unwrap_err(), "That pack has expired");
        assert_eq!(
            Source::parse(&json!({ "kind": "url", "url": "http://example.com/a.tgz" })).unwrap_err(),
            "A pack is fetched over https, or from this machine"
        );
        assert_eq!(
            Source::parse(&json!({ "kind": "url", "url": "nope" })).unwrap_err(),
            "That is not a URL a pack can be fetched from"
        );
        assert!(Source::parse(&json!({ "kind": "url", "url": "http://127.0.0.1:9/a.tgz" })).is_ok());
        assert!(Source::parse(&json!({ "kind": "url", "url": "https://h/a.tgz", "sha256": "ab" })).is_ok());
    }

    #[test]
    fn keeps_archive_entries_inside_the_root() {
        assert!(is_safe_entry("package/index.js", true));
        assert!(!is_safe_entry("../x", true));
        assert!(!is_safe_entry("a/../../x", true));
        assert!(!is_safe_entry("/etc/x", true));
        assert!(!is_safe_entry("C:x", true));
        assert!(!is_safe_entry("link", false));
    }
}

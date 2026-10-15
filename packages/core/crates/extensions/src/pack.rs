//! Installed packages on disk: `<root>/<id>/current.json` names the version
//! whose files sit in `<root>/<id>/<version>/`, or the folder a linked one
//! runs from in place.
//!
//! A package carries `vorn-extension.toml`; one built for 0.8 carries
//! `manifest.json` and `index.js` instead. Both read into the one
//! [`Manifest`]. Only reading lives here; [`crate::install`] and the 0.8 pack
//! installer write.

use std::fs;
use std::path::{Path, PathBuf};

use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;
use serde_json::Value;

use crate::manifest::{self, file, Kind, Manifest};

pub const CURRENT_FILE: &str = "current.json";
pub const MANIFEST_FILE: &str = "manifest.json";
/// What a 0.8 pack runs: `node <version dir>/index.js`.
pub const ENTRY_FILE: &str = "index.js";

/// A package as the app lists it: its manifest, where it is and when it came.
#[derive(Debug, Clone, PartialEq)]
pub struct InstalledPack {
    pub id: String,
    /// The installed version, which names the directory, not the manifest's.
    pub version: String,
    pub manifest: Manifest,
    /// Where its files are: the version's directory, or a linked folder.
    pub path: PathBuf,
    /// Run in place from a folder of the person's (`vorn ext link`).
    pub linked: bool,
    /// Where it was installed from, as written: `{ "git": url, "rev": sha }`.
    pub source: Option<Value>,
    pub previous_version: Option<String>,
    /// As written, so a pointer from another build reads back unchanged.
    pub installed_at: Value,
    pub bytes: u64,
}

/// The wire keeps 0.8's shape: the manifest's fields beside the install's.
impl Serialize for InstalledPack {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let m = &self.manifest;
        let mut map = s.serialize_map(None)?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("name", &m.name)?;
        map.serialize_entry("version", &self.version)?;
        if let Some(d) = &m.description {
            map.serialize_entry("description", d)?;
        }
        if let Some(i) = &m.icon {
            map.serialize_entry("icon", i)?;
        }
        if let Some(a) = &m.auth {
            map.serialize_entry("auth", a)?;
        }
        if let Some(p) = m.protocol {
            map.serialize_entry("protocol", &p)?;
        }
        // Written only for an extension, as the app reads a missing kind as a connector.
        if m.kind == Kind::Extension {
            map.serialize_entry("kind", &m.kind)?;
        }
        if let Some(c) = &m.contributes {
            map.serialize_entry("contributes", c)?;
        }
        if let Some(p) = &m.permissions {
            map.serialize_entry("permissions", p)?;
        }
        if let Some(a) = &m.activates {
            map.serialize_entry("activates", a)?;
        }
        map.serialize_entry("path", &self.path)?;
        if let Some(v) = &self.previous_version {
            map.serialize_entry("previousVersion", v)?;
        }
        map.serialize_entry("installedAt", &self.installed_at)?;
        map.serialize_entry("bytes", &self.bytes)?;
        map.serialize_entry("triggers", &m.triggers)?;
        map.serialize_entry("actions", &m.actions)?;
        map.serialize_entry("env", &m.env)?;
        if let Some(api) = &m.api {
            map.serialize_entry("api", api)?;
        }
        if self.linked {
            map.serialize_entry("linked", &true)?;
        }
        if let Some(source) = &self.source {
            map.serialize_entry("source", source)?;
        }
        if let Some(p) = &m.publisher {
            map.serialize_entry("publisher", p)?;
        }
        if let Some(svc) = &m.service {
            map.serialize_entry("service", svc)?;
        }
        if !m.runs.is_empty() {
            map.serialize_entry("runs", &m.runs)?;
        }
        if !m.declares.is_empty() {
            map.serialize_entry("declares", &m.declares)?;
        }
        if let Some(storage) = &m.storage {
            map.serialize_entry("storage", storage)?;
        }
        if !m.mcp.is_empty() {
            map.serialize_entry("mcp", &m.mcp)?;
        }
        map.end()
    }
}

impl InstalledPack {
    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    pub fn is_extension(&self) -> bool {
        self.manifest.kind == Kind::Extension
    }

    /// What it contributes, empty for a connector.
    pub fn contributions(&self) -> Option<&manifest::Contributions> {
        self.manifest.contributes.as_ref()
    }

    /// `argv` as it runs from this package: a program or argument naming a
    /// file inside the package becomes that file's full path, so it runs the
    /// same from any working directory.
    pub fn resolve(&self, argv: &[String]) -> Option<(PathBuf, Vec<String>)> {
        let (program, args) = argv.split_first()?;
        let inside = |arg: &str| {
            let candidate = Path::new(arg);
            let local = candidate.is_relative()
                && !arg.starts_with('-')
                && (arg.contains('/') || arg.contains('\\') || arg.contains('.'))
                && self.path.join(candidate).is_file();
            local.then(|| self.path.join(candidate))
        };
        let program = inside(program).unwrap_or_else(|| PathBuf::from(program));
        let args = args
            .iter()
            .map(|a| inside(a).map_or_else(|| a.clone(), |p| p.to_string_lossy().into_owned()))
            .collect();
        Some((program, args))
    }

    /// The folder of its tables' migrations, when it keeps tables.
    pub fn migrations(&self) -> Option<PathBuf> {
        let storage = self.manifest.storage.as_ref()?;
        Some(self.path.join(&storage.migrations))
    }

    /// The service's program, resolved; `None` when it declares none.
    pub fn service_command(&self) -> Option<(PathBuf, Vec<String>)> {
        self.resolve(&self.manifest.service.as_ref()?.command)
    }
}

/// The directory packages are installed under, read on every call: one
/// installed or removed elsewhere is seen at once.
#[derive(Debug, Clone)]
pub struct PackStore {
    root: PathBuf,
}

/// Ids are directory names, so anything that could traverse is not one.
pub fn is_safe_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_'))
        && !id.contains("..")
}

/// A version names a directory too, so it is held to the same rule.
pub fn is_safe_version(version: &str) -> bool {
    let b = version.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-'))
        && !version.contains("..")
}

/// When a package's pointer and manifest were last written, and their sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp((std::time::SystemTime, u64), (std::time::SystemTime, u64));

/// What `current.json` says.
#[derive(Debug, Clone)]
struct Current {
    version: Option<String>,
    linked: Option<PathBuf>,
    source: Option<Value>,
    previous_version: Option<String>,
    installed_at: Value,
}

/// Reads the manifest a folder carries: `vorn-extension.toml`, else a 0.8
/// pack's `manifest.json` beside the `index.js` it runs.
pub fn read_dir_manifest(dir: &Path) -> Result<Manifest, String> {
    let toml = dir.join(file::FILE);
    if toml.is_file() {
        let text = fs::read_to_string(&toml)
            .map_err(|e| format!("cannot read {}: {e}", toml.display()))?;
        return file::parse(&text).map_err(|e| e.to_string());
    }
    let json = dir.join(MANIFEST_FILE);
    let text = fs::read_to_string(&json)
        .map_err(|_| format!("{} has no {} or {MANIFEST_FILE}", dir.display(), file::FILE))?;
    let payload: Value =
        serde_json::from_str(&text).map_err(|e| format!("{MANIFEST_FILE} is not JSON: {e}"))?;
    let payload = payload
        .as_object()
        .ok_or_else(|| format!("{MANIFEST_FILE} is not an object"))?;
    if !dir.join(ENTRY_FILE).is_file() {
        return Err(format!("{} has no {ENTRY_FILE} to run", dir.display()));
    }
    file::from_pack_json(payload).map_err(|e| e.to_string())
}

impl PackStore {
    pub fn new(root: impl Into<PathBuf>) -> PackStore {
        PackStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn current(&self, id: &str) -> Option<Current> {
        let text = fs::read_to_string(self.root.join(id).join(CURRENT_FILE)).ok()?;
        let parsed: Value = serde_json::from_str(&text).ok()?;
        let linked = parsed
            .get("linked")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute());
        let version = parsed
            .get("version")
            .and_then(Value::as_str)
            .filter(|v| is_safe_version(v))
            .map(str::to_owned);
        if linked.is_none() && version.is_none() {
            return None;
        }
        Some(Current {
            version,
            linked,
            source: parsed.get("source").cloned(),
            // A bad earlier version is forgotten rather than spoiling the current one.
            previous_version: parsed
                .get("previousVersion")
                .and_then(Value::as_str)
                .filter(|v| is_safe_version(v))
                .map(str::to_owned),
            installed_at: parsed.get("installedAt").cloned().unwrap_or(Value::Null),
        })
    }

    /// The installed package `id`, or `None` when nothing usable is installed.
    pub fn describe(&self, id: &str) -> Option<InstalledPack> {
        self.inspect(id).ok()
    }

    /// The installed package `id`, or why it cannot be used.
    pub fn inspect(&self, id: &str) -> Result<InstalledPack, String> {
        self.read(id, true)
    }

    /// As [`PackStore::inspect`], without walking its files for their size:
    /// what a call needs, which must not cost a walk of `node_modules`.
    pub fn inspect_unmeasured(&self, id: &str) -> Result<InstalledPack, String> {
        self.read(id, false)
    }

    /// What `id`'s package was read from: its pointer and its manifest, by
    /// when each was last written. A package whose stamp is unchanged is unchanged.
    pub fn stamp(&self, pack: &InstalledPack) -> Option<Stamp> {
        let written = |p: PathBuf| {
            let meta = fs::metadata(p).ok()?;
            Some((meta.modified().ok()?, meta.len()))
        };
        let manifest = pack.path.join(file::FILE);
        let manifest = if manifest.is_file() {
            manifest
        } else {
            pack.path.join(MANIFEST_FILE)
        };
        Some(Stamp(
            written(self.root.join(&pack.id).join(CURRENT_FILE))?,
            written(manifest)?,
        ))
    }

    fn read(&self, id: &str, measure: bool) -> Result<InstalledPack, String> {
        if !is_safe_id(id) {
            return Err(format!("\"{id}\" is not an extension id"));
        }
        let current = self
            .current(id)
            .ok_or_else(|| format!("No extension \"{id}\" is installed"))?;
        let (path, linked) = match (&current.linked, &current.version) {
            (Some(folder), _) => (folder.clone(), true),
            (None, Some(version)) => (self.root.join(id).join(version), false),
            (None, None) => return Err(format!("No extension \"{id}\" is installed")),
        };
        let manifest = read_dir_manifest(&path)?;
        // A 0.8 pack's directory was named by its installer, not by its manifest.
        if manifest.id != id && path.join(file::FILE).is_file() {
            return Err(format!(
                "{} says its id is {}, not {id}",
                path.display(),
                manifest.id
            ));
        }
        // A linked folder holds its dependencies and history; its size says nothing.
        let bytes = if linked || !measure {
            0
        } else {
            directory_bytes(&path).map_err(|e| e.to_string())?
        };
        Ok(InstalledPack {
            id: id.to_owned(),
            version: current
                .version
                .clone()
                .unwrap_or_else(|| manifest.version.clone()),
            manifest,
            path,
            linked,
            source: current.source,
            previous_version: current.previous_version,
            installed_at: current.installed_at,
            bytes,
        })
    }

    /// The ids of every package directory, whether or not it is usable.
    pub fn ids(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| !name.starts_with('.'))
            .collect();
        ids.sort();
        ids
    }

    /// Every installed package, by name.
    ///
    /// The server sorted with `localeCompare`; this sorts by the name without
    /// case, then as written, which agrees for the names packs carry.
    pub fn list(&self) -> Vec<InstalledPack> {
        let mut packs: Vec<InstalledPack> = self
            .ids()
            .iter()
            .filter_map(|id| self.describe(id))
            .collect();
        packs.sort_by(|a, b| {
            a.name()
                .to_lowercase()
                .cmp(&b.name().to_lowercase())
                .then_with(|| a.name().cmp(b.name()))
        });
        packs
    }

    /// The installed extensions that add to a card, by name.
    pub fn extensions(&self) -> Vec<InstalledPack> {
        self.list()
            .into_iter()
            .filter(InstalledPack::is_extension)
            .collect()
    }
}

/// Bytes under `dir`, following a link to a file as `statSync` does.
pub fn directory_bytes(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        total += if entry.file_type()?.is_dir() {
            directory_bytes(&entry.path())?
        } else {
            fs::metadata(entry.path())?.len()
        };
    }
    Ok(total)
}

/// Packs written straight to disk, for tests here and in the crates built on this one.
#[cfg(test)]
pub mod fixture {
    use super::*;
    use serde_json::json;

    /// Writes an installed pack `id` at `version` with `manifest`.
    pub fn install(root: &Path, id: &str, version: &str, manifest: &Value) -> PathBuf {
        let dir = root.join(id).join(version);
        // Windows resolves `id/../x` without creating `id`, where current.json goes.
        fs::create_dir_all(root.join(id)).unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(ENTRY_FILE), "process.stdin.resume()\n").unwrap();
        fs::write(dir.join(MANIFEST_FILE), manifest.to_string()).unwrap();
        fs::write(
            root.join(id).join(CURRENT_FILE),
            json!({ "version": version, "previousVersion": "../x", "installedAt": 5 }).to_string(),
        )
        .unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::install;
    use super::*;
    use serde_json::json;

    fn extension(name: &str) -> Value {
        json!({ "id": name, "name": name, "kind": "extension", "protocol": 1,
                "contributes": { "footers": [{ "id": "f", "every": 5 }] } })
    }

    #[test]
    fn describes_an_installed_extension() {
        let root = tempfile::tempdir().unwrap();
        let dir = install(root.path(), "ext", "1.0.0", &extension("Ext"));
        let pack = PackStore::new(root.path()).describe("ext").unwrap();
        assert!(pack.is_extension());
        let (program, args) = pack.service_command().unwrap();
        assert_eq!(program, PathBuf::from("node"));
        assert_eq!(args, [dir.join("index.js").to_string_lossy()]);
        assert_eq!(pack.previous_version, None);
        let out = serde_json::to_value(&pack).unwrap();
        let keys: Vec<&str> = out
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "id",
                "name",
                "version",
                "protocol",
                "kind",
                "contributes",
                "path",
                "installedAt",
                "bytes",
                "triggers",
                "actions",
                "env",
                "api",
                "service"
            ]
        );
        assert!(out["bytes"].as_u64().unwrap() > 0);
    }

    #[test]
    fn lists_by_name_and_skips_what_is_not_installed() {
        let root = tempfile::tempdir().unwrap();
        install(root.path(), "b", "1.0.0", &extension("beta"));
        install(root.path(), "a", "1.0.0", &extension("Alpha"));
        install(
            root.path(),
            "c",
            "1.0.0",
            &json!({ "id": "c", "name": "Conn", "actions": [{ "type": "go" }] }),
        );
        install(root.path(), "d", "1.0.0", &json!({ "id": "d" }));
        install(root.path(), "e", "../up", &extension("E"));
        fs::create_dir_all(root.path().join(".staging")).unwrap();
        let store = PackStore::new(root.path());
        let names: Vec<String> = store
            .list()
            .into_iter()
            .map(|p| p.name().to_owned())
            .collect();
        assert_eq!(names, ["Alpha", "beta", "Conn"]);
        assert_eq!(store.extensions().len(), 2);
        assert!(store.describe("../a").is_none());
        assert!(PackStore::new(root.path().join("none")).list().is_empty());
    }

    #[test]
    fn needs_the_file_it_runs() {
        let root = tempfile::tempdir().unwrap();
        let dir = install(root.path(), "ext", "1.0.0", &extension("Ext"));
        fs::remove_file(dir.join(ENTRY_FILE)).unwrap();
        assert!(PackStore::new(root.path()).describe("ext").is_none());
    }

    #[test]
    fn reads_a_linked_folder_in_place() {
        let root = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join(file::FILE),
            "id = \"local.hello\"\nname = \"Hello\"\nversion = \"0.1.0\"\napi = \"vorn:extension@0.9\"\n[actions.greet]\ncommand = [\"bash\", \"greet.sh\"]\n",
        )
        .unwrap();
        fs::write(folder.path().join("greet.sh"), "echo hi\n").unwrap();
        fs::create_dir_all(root.path().join("local.hello")).unwrap();
        fs::write(
            root.path().join("local.hello").join(CURRENT_FILE),
            json!({ "linked": folder.path(), "installedAt": 1 }).to_string(),
        )
        .unwrap();
        let store = PackStore::new(root.path());
        let pack = store.inspect("local.hello").unwrap();
        assert!(pack.linked);
        assert_eq!((pack.version.as_str(), pack.bytes), ("0.1.0", 0));
        let run = pack.manifest.run("greet").unwrap();
        let (program, args) = pack.resolve(&run.command).unwrap();
        assert_eq!(program, PathBuf::from("bash"));
        assert_eq!(args, [folder.path().join("greet.sh").to_string_lossy()]);
        let out = serde_json::to_value(&pack).unwrap();
        assert_eq!(out["linked"], true);
        assert_eq!(out["runs"][0]["id"], "greet");
        // A folder whose manifest names another id is not that extension.
        fs::create_dir_all(root.path().join("other")).unwrap();
        fs::write(
            root.path().join("other").join(CURRENT_FILE),
            json!({ "linked": folder.path() }).to_string(),
        )
        .unwrap();
        assert!(store
            .inspect("other")
            .unwrap_err()
            .contains("says its id is local.hello"));
    }

    #[test]
    fn holds_ids_and_versions_to_path_segments() {
        assert!(is_safe_id("my-ext1") && is_safe_id("example.pr_checks"));
        assert!(!is_safe_id(".x") && !is_safe_id("a..b"));
        assert!(
            !is_safe_id("-x")
                && !is_safe_id("")
                && !is_safe_id("a/b")
                && !is_safe_id(&"a".repeat(65))
        );
        assert!(is_safe_version("1.0.0-beta+1"));
        assert!(!is_safe_version("1..0") && !is_safe_version(".1"));
    }
}

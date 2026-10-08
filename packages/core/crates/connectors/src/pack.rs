//! Installed packs on disk: `<root>/<id>/current.json` names the version
//! whose files sit in `<root>/<id>/<version>/`.
//!
//! Only reading lives here; installing, removing and rolling back are the
//! connector side's, which tells the host when a pack changed.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::manifest::{self, Activation, Auth, Contributions, Icon, Kind, Manifest, Permission};

const CURRENT_FILE: &str = "current.json";
const MANIFEST_FILE: &str = "manifest.json";
/// What a pack runs: `node <version dir>/index.js`.
pub const ENTRY_FILE: &str = "index.js";

/// A pack as the app lists it: its manifest, where it is and when it came.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPack {
    pub id: String,
    pub name: String,
    /// The installed version, which names the directory, not the manifest's.
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u64>,
    /// Written only for an extension, as the app reads a missing kind as a connector.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributes: Option<Contributions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<Permission>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activates: Option<Activation>,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    /// As written, so a pointer from another build reads back unchanged.
    pub installed_at: Value,
    pub bytes: u64,
    pub triggers: Vec<manifest::Trigger>,
    pub actions: Vec<manifest::Action>,
    pub env: Vec<manifest::EnvVar>,
}

impl InstalledPack {
    pub fn is_extension(&self) -> bool {
        self.kind == Some(Kind::Extension)
    }

    /// The file a pack's child runs.
    pub fn entry(&self) -> PathBuf {
        self.path.join(ENTRY_FILE)
    }

    /// What it contributes, empty for a connector.
    pub fn contributions(&self) -> Option<&Contributions> {
        self.contributes.as_ref()
    }
}

/// The directory packs are installed under, read on every call: a pack
/// installed or removed by the connector side is seen at once.
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
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
}

/// A version names a directory too, so it is held to the same rule.
fn is_safe_version(version: &str) -> bool {
    let b = version.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-'))
        && !version.contains("..")
}

struct Current {
    version: String,
    previous_version: Option<String>,
    installed_at: Value,
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
        let version = parsed
            .get("version")?
            .as_str()
            .filter(|v| is_safe_version(v))?;
        Some(Current {
            version: version.to_owned(),
            // A bad earlier version is forgotten rather than spoiling the current one.
            previous_version: parsed
                .get("previousVersion")
                .and_then(Value::as_str)
                .filter(|v| is_safe_version(v))
                .map(str::to_owned),
            installed_at: parsed.get("installedAt").cloned().unwrap_or(Value::Null),
        })
    }

    /// The installed pack `id`, or `None` when nothing usable is installed.
    pub fn describe(&self, id: &str) -> Option<InstalledPack> {
        if !is_safe_id(id) {
            return None;
        }
        let current = self.current(id)?;
        let path = self.root.join(id).join(&current.version);
        if !path.join(ENTRY_FILE).exists() {
            return None;
        }
        let manifest = read_manifest(&path)?;
        let extension = manifest.kind == Kind::Extension;
        Some(InstalledPack {
            id: id.to_owned(),
            name: manifest.name,
            version: current.version,
            description: manifest.description,
            icon: manifest.icon,
            auth: manifest.auth,
            protocol: manifest.protocol,
            kind: extension.then_some(Kind::Extension),
            contributes: manifest.contributes,
            permissions: manifest.permissions,
            activates: manifest.activates,
            bytes: directory_bytes(&path).ok()?,
            path,
            previous_version: current.previous_version,
            installed_at: current.installed_at,
            triggers: manifest.triggers,
            actions: manifest.actions,
            env: manifest.env,
        })
    }

    /// Every installed pack, by name.
    ///
    /// The server sorts with `localeCompare`; this sorts by the name without
    /// case, then as written, which agrees for the names packs carry.
    pub fn list(&self) -> Vec<InstalledPack> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut packs: Vec<InstalledPack> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| !name.starts_with('.'))
            .filter_map(|name| self.describe(&name))
            .collect();
        packs.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
        packs
    }

    /// The installed extensions, by name.
    pub fn extensions(&self) -> Vec<InstalledPack> {
        self.list()
            .into_iter()
            .filter(InstalledPack::is_extension)
            .collect()
    }
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    let text = fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    let payload: Value = serde_json::from_str(&text).ok()?;
    manifest::read(payload.as_object()?).ok()
}

/// Bytes under `dir`, following a link to a file as `statSync` does.
fn directory_bytes(dir: &Path) -> std::io::Result<u64> {
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
#[cfg(any(test, feature = "test-support"))]
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
        assert_eq!(pack.entry(), dir.join("index.js"));
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
                "env"
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
        let names: Vec<String> = store.list().into_iter().map(|p| p.name).collect();
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
    fn holds_ids_and_versions_to_path_segments() {
        assert!(is_safe_id("my-ext1"));
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

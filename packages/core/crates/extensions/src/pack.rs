//! Installed packages on disk: `<root>/<id>/current.json` names the version
//! whose files sit in `<root>/<id>/<version>/`, or the folder a linked one
//! runs from in place.
//!
//! A package carries `vorn-extension.toml`. Only reading lives here;
//! [`crate::install`] writes.

use std::fs;
use std::path::{Path, PathBuf};

use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;
use serde_json::Value;

use crate::manifest::{self, file, Kind, Manifest};

pub const CURRENT_FILE: &str = "current.json";

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
    /// When it was first installed, kept by updates; as written, so a
    /// pointer from another build reads back unchanged.
    pub installed_at: Value,
    /// The agent a linked folder adapted when it was linked; one it adapts
    /// since was claimed after every other adapter's.
    pub linked_agent: Option<String>,
    /// The sha256 of a linked folder's manifest when it was linked: one
    /// edited since claimed its agent after every other adapter.
    pub linked_manifest: Option<String>,
    pub bytes: u64,
}

/// On the wire, the manifest's fields sit beside the install's.
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
        let tags = m.tags();
        if !tags.is_empty() {
            map.serialize_entry("tags", &tags)?;
        }
        if !m.showcase.is_empty() {
            map.serialize_entry("showcase", &m.showcase)?;
        }
        map.end()
    }
}

/// The most bytes of a logo file sent to the app; a logo is a small drawing.
const MAX_LOGO_BYTES: u64 = 64 * 1024;

impl InstalledPack {
    /// Its logo's SVG text, or with `dark` its dark variant's; `None` when it
    /// has none or the file is missing, too big or not SVG.
    pub fn logo_svg(&self, dark: bool) -> Option<String> {
        let file = self.manifest.logo_file(dark)?;
        let inside = Path::new(file).components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        });
        let path = self.path.join(file);
        if !inside || fs::metadata(&path).ok()?.len() > MAX_LOGO_BYTES {
            return None;
        }
        let text = fs::read_to_string(path).ok()?;
        text.contains("<svg").then_some(text)
    }
}

impl InstalledPack {
    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    /// Whether it adds anything to Vorn's surfaces; a connector may too.
    pub fn is_extension(&self) -> bool {
        self.manifest.contributes.is_some()
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

    /// Its tables' migrations, when it keeps tables.
    pub fn migrations(&self) -> Option<Migrations> {
        let storage = self.manifest.storage.as_ref()?;
        Some(Migrations {
            folder: self.path.clone(),
            dir: storage.migrations.clone(),
        })
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
/// Lowercase only, so no two ids name one folder on a case-insensitive disk.
pub fn is_safe_id(id: &str) -> bool {
    let lower = |c: &u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && lower(&b[0])
        && b.iter()
            .all(|c| lower(c) || matches!(c, b'-' | b'.' | b'_'))
        && !id.contains("..")
}

/// Ids Vorn's own workflow ids start with (`import:`, `connector:`,
/// `system:`, and `file:` for a repository's): an extension of that id would own them.
pub const RESERVED_IDS: [&str; 4] = ["import", "connector", "system", "file"];

/// The namespace of the extensions Vorn ships: one installed under such an
/// id would own its workflows (`vorn.connectors:` and the rest).
pub const SHIPPED_NAMESPACE: &str = "vorn.";

/// Whether `id` is Vorn's: only a pack whose checksum Vorn pinned may have
/// it (see [`crate::install::Installs::inspect_archive`]).
/// Compared in lowercase, as a case-insensitive file system names its folder.
pub fn is_reserved(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    RESERVED_IDS.contains(&id.as_str()) || id.starts_with(SHIPPED_NAMESPACE)
}

/// Refuses an id an extension may not have: unsafe as a directory, or
/// reserved for Vorn's own extensions and workflows.
pub fn check_id(id: &str) -> Result<(), String> {
    if !is_safe_id(id) {
        return Err(format!("\"{id}\" is not an extension id"));
    }
    if id.starts_with(SHIPPED_NAMESPACE) {
        return Err(format!(
            "\"{id}\" is reserved: ids starting with {SHIPPED_NAMESPACE} are the extensions Vorn ships"
        ));
    }
    if is_reserved(id) {
        return Err(format!(
            "\"{id}\" is reserved: Vorn's own workflows' ids start with {id}:"
        ));
    }
    Ok(())
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
    linked_agent: Option<String>,
    linked_manifest: Option<String>,
    /// Why a linked folder is not loaded: it gained a link since it was linked.
    refused: Option<String>,
}

/// The key in `current.json` saying why a linked folder is not loaded.
pub const REFUSED_KEY: &str = "refused";

/// Reads the manifest a folder carries, `vorn-extension.toml`.
pub fn read_dir_manifest(dir: &Path) -> Result<Manifest, String> {
    let toml = dir.join(file::FILE);
    if !toml.is_file() {
        return Err(format!("{} has no {}", dir.display(), file::FILE));
    }
    let text =
        fs::read_to_string(&toml).map_err(|e| format!("cannot read {}: {e}", toml.display()))?;
    file::parse(&text).map_err(|e| e.to_string())
}

/// Relabels the manifest in `dir` from a pre-1.0 API to this Vorn's
/// ([`crate::compat::relabel`]); whether it wrote. Install runs it on an
/// unpacked package and vornd on every installed one at start, the one
/// rewrite for both.
pub fn relabel_dir(dir: &Path) -> Result<bool, String> {
    use std::io::Write;
    let toml = dir.join(file::FILE);
    let refused = |e: &dyn std::fmt::Display| format!("cannot relabel {}: {e}", toml.display());
    match fs::symlink_metadata(&toml) {
        Ok(m) if m.file_type().is_symlink() => return Err(refused(&"it is a symbolic link")),
        Ok(_) => {}
        Err(_) => return Ok(false),
    }
    let Ok(text) = fs::read_to_string(&toml) else {
        return Ok(false);
    };
    let Some(next) = crate::compat::relabel(&text) else {
        return Ok(false);
    };
    // create_new refuses any path already there, a symbolic link included,
    // so a folder from a repository cannot aim the write elsewhere.
    let tmp = dir.join(format!(".{}.{}.tmp", file::FILE, std::process::id()));
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| refused(&e))?;
    let written = out
        .write_all(next.as_bytes())
        .and_then(|()| out.sync_all())
        .and_then(|()| fs::rename(&tmp, &toml));
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(refused(&e));
    }
    Ok(true)
}

impl PackStore {
    /// Relabels every installed version's manifest written for the pre-1.0
    /// API; a linked folder is the person's own and is left alone. Answers
    /// the folders it rewrote, and the ones it could not with why.
    pub fn relabel_installed(&self) -> (Vec<PathBuf>, Vec<String>) {
        let (mut done, mut failed) = (Vec::new(), Vec::new());
        for id in self.ids() {
            let Ok(entries) = fs::read_dir(self.root.join(&id)) else {
                continue;
            };
            for entry in entries.flatten() {
                let dir = entry.path();
                let named = entry.file_name().to_str().is_some_and(is_safe_version);
                if !named || !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                match relabel_dir(&dir) {
                    Ok(true) => done.push(dir),
                    Ok(false) => {}
                    Err(why) => failed.push(why),
                }
            }
        }
        (done, failed)
    }

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
            linked_agent: parsed
                .get("linkedAgent")
                .and_then(Value::as_str)
                .map(str::to_owned),
            linked_manifest: parsed
                .get("linkedManifest")
                .and_then(Value::as_str)
                .map(str::to_owned),
            refused: parsed
                .get(REFUSED_KEY)
                .and_then(Value::as_str)
                .map(str::to_owned),
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
        if let (Some(folder), Some(why)) = (&current.linked, &current.refused) {
            return Err(format!(
                "{id} is not loaded from {}: {why}",
                folder.display()
            ));
        }
        let (path, linked) = match (&current.linked, &current.version) {
            (Some(folder), _) => (folder.clone(), true),
            (None, Some(version)) => (self.root.join(id).join(version), false),
            (None, None) => return Err(format!("No extension \"{id}\" is installed")),
        };
        let manifest = read_dir_manifest(&path)?;
        if manifest.id != id {
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
            linked_agent: current.linked_agent,
            linked_manifest: current.linked_manifest,
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

/// A package's migrations folder, listed and read without following any
/// link below the package, so a link put in it after it was checked is
/// refused rather than run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrations {
    folder: PathBuf,
    dir: String,
}

impl Migrations {
    /// The names of the files in it; none when it is missing or not a folder.
    pub fn names(&self) -> std::io::Result<Vec<String>> {
        match crate::reach::list_in(&self.folder, &self.dir) {
            Ok(entries) => Ok(entries
                .into_iter()
                .filter(|(_, is_dir)| !is_dir)
                .filter_map(|(name, _)| name.into_string().ok())
                .collect()),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound
                        | std::io::ErrorKind::NotADirectory
                        | std::io::ErrorKind::InvalidInput
                ) =>
            {
                Ok(Vec::new())
            }
            Err(e) => Err(e),
        }
    }

    /// The text of the migration `name` in it.
    pub fn read(&self, name: &str) -> std::io::Result<String> {
        let bytes = crate::reach::read_in(&self.folder, &format!("{}/{name}", self.dir))?;
        String::from_utf8(bytes).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{name} is not UTF-8"),
            )
        })
    }
}

/// Packages written straight to disk, for tests here.
#[cfg(test)]
pub mod fixture {
    use super::*;
    use serde_json::json;

    /// Writes an installed package `id` at `version`: `vorn-extension.toml`
    /// holds its id, version and api, then `body` (its name and sections).
    pub fn install(root: &Path, id: &str, version: &str, body: &str) -> PathBuf {
        let dir = root.join(id).join(version);
        // Windows resolves `id/../x` without creating `id`, where current.json goes.
        fs::create_dir_all(root.join(id)).unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.js"), "process.stdin.resume()\n").unwrap();
        let api = manifest::API;
        let toml = format!("id = \"{id}\"\nversion = \"1.0.0\"\napi = \"{api}\"\n{body}");
        fs::write(dir.join(file::FILE), toml).unwrap();
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

    fn extension(name: &str) -> String {
        format!(
            "name = \"{name}\"\n[service]\ncommand = [\"node\", \"index.js\"]\n[connector]\n[[contributes.footers]]\nid = \"f\"\nevery = 5.0\n"
        )
    }

    #[test]
    fn sends_its_logo_and_tags() {
        let root = tempfile::tempdir().unwrap();
        let body = "name = \"Acme\"\n[icon]\nfile = \"logo.svg\"\ndark = \"dark.svg\"\n[service]\ncommand = [\"node\", \"index.js\"]\n[connector]\n";
        let dir = install(root.path(), "acme", "1.0.0", body);
        fs::write(dir.join("logo.svg"), "<svg id=\"light\"/>").unwrap();
        fs::write(dir.join("dark.svg"), "not a drawing").unwrap();
        let pack = PackStore::new(root.path()).inspect("acme").unwrap();
        assert_eq!(pack.logo_svg(false).as_deref(), Some("<svg id=\"light\"/>"));
        assert_eq!(pack.logo_svg(true), None);
        let wire = serde_json::to_value(&pack).unwrap();
        assert_eq!(wire["tags"], json!(["connection"]));
    }

    #[test]
    fn installed_packages_written_for_0_9_are_relabelled_and_read_again() {
        let root = tempfile::tempdir().unwrap();
        let dir = install(root.path(), "old", "1.0.0", &extension("Old"));
        let toml = dir.join(file::FILE);
        let text = fs::read_to_string(&toml).unwrap();
        fs::write(&toml, text.replace(manifest::API, "vorn:extension@0.9")).unwrap();
        let linked = tempfile::tempdir().unwrap();
        fs::write(
            linked.path().join(file::FILE),
            "api = \"vorn:extension@0.9\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join("mine")).unwrap();
        fs::write(
            root.path().join("mine").join(CURRENT_FILE),
            json!({ "linked": linked.path() }).to_string(),
        )
        .unwrap();
        let store = PackStore::new(root.path());
        assert!(store.describe("old").is_none(), "0.9 is refused as it is");

        let (done, failed) = store.relabel_installed();
        assert_eq!((done, failed), (vec![dir], Vec::<String>::new()));
        assert_eq!(fs::read_to_string(&toml).unwrap(), text);
        assert!(store.describe("old").is_some());
        assert_eq!(
            fs::read_to_string(linked.path().join(file::FILE)).unwrap(),
            "api = \"vorn:extension@0.9\"\n",
            "a linked folder is the person's own"
        );
        assert_eq!(store.relabel_installed().0, Vec::<PathBuf>::new());
    }

    #[cfg(unix)]
    #[test]
    fn relabelling_follows_no_symbolic_link() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("rc");
        fs::write(&target, "mine\n").unwrap();
        let old = "api = \"vorn:extension@0.9\"\n";

        // A temp file the repository planted as a link is never written through.
        let repo = tempfile::tempdir().unwrap();
        fs::write(repo.path().join(file::FILE), old).unwrap();
        let planted = format!(".{}.{}.tmp", file::FILE, std::process::id());
        symlink(&target, repo.path().join(&planted)).unwrap();
        symlink(&target, repo.path().join(format!("{}.tmp", file::FILE))).unwrap();
        assert!(relabel_dir(repo.path()).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "mine\n");
        assert_eq!(
            fs::read_to_string(repo.path().join(file::FILE)).unwrap(),
            old
        );

        // Nor is a manifest that is itself a link.
        let repo = tempfile::tempdir().unwrap();
        fs::write(&target, old).unwrap();
        symlink(&target, repo.path().join(file::FILE)).unwrap();
        let refused = relabel_dir(repo.path()).unwrap_err();
        assert!(refused.ends_with("it is a symbolic link"), "{refused}");
        assert_eq!(fs::read_to_string(&target).unwrap(), old);
    }

    #[test]
    fn ids_vorn_names_its_own_workflows_by_are_reserved() {
        for id in RESERVED_IDS {
            let refused = check_id(id).unwrap_err();
            assert_eq!(
                refused,
                format!("\"{id}\" is reserved: Vorn's own workflows' ids start with {id}:")
            );
        }
        for id in ["vorn.connectors", "vorn.workflows", "vorn.anything"] {
            assert_eq!(
                check_id(id).unwrap_err(),
                format!(
                    "\"{id}\" is reserved: ids starting with vorn. are the extensions Vorn ships"
                )
            );
        }
        assert!(check_id("importer").is_ok() && check_id("vornish.x").is_ok());
        assert!(check_id("a/b").is_err());
    }

    #[test]
    fn ids_are_lowercase_only() {
        for id in ["Import", "SYSTEM", "Vorn.Connectors", "Test.Marker", "a.B"] {
            assert!(!is_safe_id(id), "{id}");
            assert_eq!(
                check_id(id).unwrap_err(),
                format!("\"{id}\" is not an extension id")
            );
        }
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
                "permissions",
                "path",
                "installedAt",
                "bytes",
                "triggers",
                "actions",
                "env",
                "api",
                "service",
                "tags"
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
            "name = \"Conn\"\n[[connector.actions]]\ntype = \"go\"\n",
        );
        // No name, so it does not read.
        install(root.path(), "d", "1.0.0", "");
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
    fn needs_its_manifest() {
        let root = tempfile::tempdir().unwrap();
        let dir = install(root.path(), "ext", "1.0.0", &extension("Ext"));
        fs::remove_file(dir.join(file::FILE)).unwrap();
        fs::write(dir.join("manifest.json"), r#"{"id":"ext","name":"Ext"}"#).unwrap();
        let err = PackStore::new(root.path()).inspect("ext").unwrap_err();
        assert!(err.ends_with("has no vorn-extension.toml"), "{err}");
    }

    #[test]
    fn reads_a_linked_folder_in_place() {
        let root = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join(file::FILE),
            "id = \"local.hello\"\nname = \"Hello\"\nversion = \"0.1.0\"\napi = \"vorn:extension@1.0\"\n[actions.greet]\ncommand = [\"bash\", \"greet.sh\"]\n",
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

    #[cfg(unix)]
    #[test]
    fn migrations_are_listed_and_read_without_following_a_link() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("any.sql"), "DROP TABLE tasks;").unwrap();
        let folder = tempfile::tempdir().unwrap();
        let of = |dir: &str| Migrations {
            folder: folder.path().to_path_buf(),
            dir: dir.into(),
        };
        assert!(of("migrations").names().unwrap().is_empty(), "none yet");
        fs::create_dir(folder.path().join("migrations")).unwrap();
        fs::write(folder.path().join("migrations/001.sql"), "SELECT 1;").unwrap();
        fs::create_dir(folder.path().join("migrations/old")).unwrap();
        assert_eq!(of("migrations").names().unwrap(), ["001.sql"]);
        assert_eq!(of("migrations").read("001.sql").unwrap(), "SELECT 1;");
        assert!(of("migrations/001.sql").names().unwrap().is_empty());

        std::os::unix::fs::symlink(
            outside.path().join("any.sql"),
            folder.path().join("migrations/002.sql"),
        )
        .unwrap();
        let err = of("migrations").read("002.sql").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
        std::os::unix::fs::symlink(outside.path(), folder.path().join("linked")).unwrap();
        let err = of("linked").names().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
    }
}

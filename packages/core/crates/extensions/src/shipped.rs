//! The shipped set: the extensions the installer carries so a first launch
//! works offline, installed through the same path as any other.
//!
//! Each is a pack whose version and sha256 Vorn pinned. [`sync`] installs the
//! ones a person has not removed and replaces an older shipped or catalog
//! install with the carried version; one installed from git or linked is the
//! person's and is left alone. A removal is kept in `shipped.json` beside the
//! packages, so the next start does not put it back.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;

use crate::install::{Installs, Pinned};

/// One extension the installer carries.
#[derive(Debug, Clone, Copy)]
pub struct Shipped<'a> {
    pub id: &'a str,
    pub version: &'a str,
    /// The pack's sha256, in lowercase hex, as the lock pins it.
    pub sha256: &'a str,
    /// Installed on a first launch; one that is not is offered in the catalog only.
    pub default: bool,
    /// The `.vorn.tgz` itself.
    pub bytes: &'a [u8],
}

impl Shipped<'_> {
    fn pinned(&self) -> Pinned {
        Pinned {
            id: self.id.to_owned(),
            version: self.version.to_owned(),
            sha256: self.sha256.to_owned(),
        }
    }
}

/// What a [`sync`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Synced {
    pub installed: Vec<String>,
    pub updated: Vec<String>,
}

/// What `shipped.json` keeps: the shipped extensions the person removed.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    declined: BTreeSet<String>,
}

/// The shipped set's record beside the packages in `root`.
#[derive(Debug, Clone)]
pub struct Record {
    file: PathBuf,
    /// The built-ins' switches the moved extensions had, read once.
    switches: PathBuf,
}

impl Record {
    /// The record for packages kept in `packages`: `shipped.json` beside them.
    pub fn beside(packages: &Path) -> Record {
        let dir = packages.parent().unwrap_or(packages);
        Record {
            file: dir.join("shipped.json"),
            switches: dir.join("builtins.json"),
        }
    }

    fn read(&self) -> Option<State> {
        let bytes = fs::read(&self.file).ok()?;
        Some(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    fn write(&self, state: &State) {
        let text = serde_json::to_vec_pretty(state).unwrap_or_default();
        if let Err(err) = fs::write(&self.file, text) {
            warn!("[extensions] could not keep {}: {err}", self.file.display());
        }
    }

    /// Keeps that the person removed `id`, so no start puts it back.
    pub fn decline(&self, id: &str) {
        let mut state = self.read().unwrap_or_default();
        if state.declined.insert(id.to_owned()) {
            self.write(&state);
        }
    }

    /// Forgets a removal of `id`, for when the person installs it again.
    pub fn accept(&self, id: &str) {
        if let Some(mut state) = self.read() {
            if state.declined.remove(id) {
                self.write(&state);
            }
        }
    }

    /// The record, made once from the built-ins' switches the shipped set
    /// had before it installed like any extension: off becomes removed, and
    /// on installs it even when the set does not by default (returned).
    fn load(&self, set: &[Shipped<'_>]) -> (State, BTreeSet<String>) {
        if let Some(state) = self.read() {
            return (state, BTreeSet::new());
        }
        let mut switches: BTreeMap<String, bool> = fs::read(&self.switches)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let (mut state, mut on) = (State::default(), BTreeSet::new());
        for s in set {
            match switches.remove(s.id) {
                Some(false) => {
                    state.declined.insert(s.id.to_owned());
                }
                Some(true) => {
                    on.insert(s.id.to_owned());
                }
                None => {}
            }
        }
        if !state.declined.is_empty() || !on.is_empty() {
            let text = serde_json::to_vec_pretty(&switches).unwrap_or_default();
            if let Err(err) = fs::write(&self.switches, text) {
                warn!("[extensions] could not rewrite the built-ins' switches: {err}");
            }
        }
        self.write(&state);
        (state, on)
    }
}

/// Brings the installed packs in line with `set`: installs each one not
/// removed and not yet installed (on a first launch only those installed by
/// default), and replaces an older shipped or catalog install. Runs nothing of
/// any pack; a failure is logged and leaves the rest to install.
pub fn sync(installs: &Installs, set: &[Shipped<'_>], record: &Record) -> Synced {
    let (state, on) = record.load(set);
    let mut synced = Synced::default();
    for s in set {
        if state.declined.contains(s.id) {
            continue;
        }
        let update = match installs.store().describe(s.id) {
            None if s.default || on.contains(s.id) => false,
            None => continue,
            Some(pack) if pack.linked || !replaceable(pack.source.as_ref()) => continue,
            Some(pack) if !older(&pack.version, s.version) => continue,
            Some(_) => true,
        };
        match install(installs, s) {
            Ok(()) if update => synced.updated.push(s.id.to_owned()),
            Ok(()) => synced.installed.push(s.id.to_owned()),
            Err(err) => warn!("[extensions] {} {} did not install: {err}", s.id, s.version),
        }
    }
    synced
}

/// Installs `s` from the bytes the installer carries.
pub fn install(installs: &Installs, s: &Shipped<'_>) -> Result<(), String> {
    let source = json!({ "shipped": s.version });
    let sheet = installs.inspect_archive(s.bytes, source, Some(&s.pinned()))?;
    installs.confirm(&sheet.token, s.id).map(drop)
}

/// Whether an install came from Vorn: the shipped set or the catalog's pack.
fn replaceable(source: Option<&serde_json::Value>) -> bool {
    source.is_some_and(|s| s.get("shipped").is_some() || s.get("catalog").is_some())
}

fn older(installed: &str, shipped: &str) -> bool {
    match (
        semver::Version::parse(installed),
        semver::Version::parse(shipped),
    ) {
        (Ok(a), Ok(b)) => a < b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::sha256_hex;
    use crate::pack::PackStore;

    fn tgz(id: &str, version: &str) -> Vec<u8> {
        let manifest = format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi = \"vorn:extension@1.0\"\npublisher = \"Vorn\"\n\n[actions.hello]\ntitle = \"Hello\"\ncommand = [\"sh\", \"hello.sh\"]\n"
        );
        let mut tar = tar::Builder::new(Vec::new());
        for (path, bytes) in [
            (crate::manifest::file::FILE, manifest.as_bytes()),
            ("hello.sh", b"true\n".as_slice()),
        ] {
            let mut header = tar::Header::new_ustar();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            tar.append_data(&mut header, format!("{id}-{version}/{path}"), bytes)
                .unwrap();
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &tar.into_inner().unwrap()).unwrap();
        gz.finish().unwrap()
    }

    struct Fixture {
        dir: tempfile::TempDir,
        installs: Installs,
        record: Record,
        packs: Vec<(String, String, Vec<u8>, bool)>,
    }

    impl Fixture {
        fn new(packs: &[(&str, &str, bool)]) -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("packages");
            let installs = Installs::new(PackStore::new(&root), PathBuf::from("git"), Vec::new());
            Fixture {
                record: Record::beside(&root),
                installs,
                packs: packs
                    .iter()
                    .map(|(id, v, d)| ((*id).into(), (*v).into(), tgz(id, v), *d))
                    .collect(),
                dir,
            }
        }

        fn set(&self) -> Vec<Shipped<'_>> {
            self.packs
                .iter()
                .map(|(id, version, bytes, default)| Shipped {
                    id,
                    version,
                    sha256: Box::leak(sha256_hex(bytes).into_boxed_str()),
                    default: *default,
                    bytes,
                })
                .collect()
        }

        fn sync(&self) -> Synced {
            sync(&self.installs, &self.set(), &self.record)
        }

        fn version(&self, id: &str) -> Option<String> {
            self.installs.store().describe(id).map(|p| p.version)
        }
    }

    #[test]
    fn a_first_launch_installs_the_default_set_offline() {
        let f = Fixture::new(&[("vorn.a", "1.0.0", true), ("vorn.b", "1.0.0", false)]);
        let synced = f.sync();
        assert_eq!(synced.installed, ["vorn.a"]);
        assert_eq!(f.version("vorn.a").as_deref(), Some("1.0.0"));
        assert_eq!(f.version("vorn.b"), None);
        let source = f.installs.store().describe("vorn.a").unwrap().source.unwrap();
        assert_eq!(source["shipped"], "1.0.0");
        assert_eq!(f.sync(), Synced::default(), "a second start changes nothing");
    }

    #[test]
    fn a_removal_is_kept_and_a_reinstall_forgets_it() {
        let f = Fixture::new(&[("vorn.a", "1.0.0", true)]);
        f.sync();
        f.installs.remove("vorn.a").unwrap();
        assert_eq!(f.sync(), Synced::default());
        assert_eq!(f.version("vorn.a"), None);
        let left: Vec<_> = fs::read_dir(f.dir.path().join("packages"))
            .unwrap()
            .flatten()
            .collect();
        assert!(left.is_empty(), "{left:?}");

        install(&f.installs, &f.set()[0]).unwrap();
        assert!(f.record.read().unwrap().declined.is_empty());
        assert_eq!(f.version("vorn.a").as_deref(), Some("1.0.0"));
    }

    #[test]
    fn a_newer_shipped_version_replaces_an_older_one_and_keeps_the_previous() {
        let old = Fixture::new(&[("vorn.a", "1.0.0", true)]);
        old.sync();
        let new = Fixture {
            packs: vec![("vorn.a".into(), "1.1.0".into(), tgz("vorn.a", "1.1.0"), true)],
            ..old
        };
        assert_eq!(new.sync().updated, ["vorn.a"]);
        let pack = new.installs.store().describe("vorn.a").unwrap();
        assert_eq!(pack.version, "1.1.0");
        assert_eq!(pack.previous_version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn an_install_of_the_persons_own_is_left_alone() {
        let f = Fixture::new(&[("vorn.a", "2.0.0", true)]);
        let older = Fixture::new(&[("vorn.a", "1.0.0", true)]);
        let pack = &older.set()[0];
        let sheet = f
            .installs
            .inspect_archive(pack.bytes, json!({ "git": "x" }), Some(&pack.pinned()))
            .unwrap();
        f.installs.confirm(&sheet.token, "vorn.a").unwrap();
        assert_eq!(f.sync(), Synced::default());
        assert_eq!(f.version("vorn.a").as_deref(), Some("1.0.0"));
    }

    #[test]
    fn the_old_switches_move_once() {
        let f = Fixture::new(&[
            ("vorn.a", "1.0.0", true),
            ("vorn.b", "1.0.0", false),
            ("vorn.c", "1.0.0", true),
        ]);
        let switches = f.dir.path().join("builtins.json");
        fs::write(
            &switches,
            r#"{"vorn.a": false, "vorn.b": true, "vorn.files": false}"#,
        )
        .unwrap();
        let synced = f.sync();
        assert_eq!(synced.installed, ["vorn.b", "vorn.c"]);
        let kept: BTreeMap<String, bool> =
            serde_json::from_slice(&fs::read(&switches).unwrap()).unwrap();
        assert_eq!(kept, BTreeMap::from([("vorn.files".to_owned(), false)]));
        let state: State =
            serde_json::from_slice(&fs::read(f.dir.path().join("shipped.json")).unwrap())
                .unwrap();
        assert_eq!(state.declined, BTreeSet::from(["vorn.a".to_owned()]));
    }
}

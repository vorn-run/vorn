//! Where an extension's `files.*` calls reach, and what a person handed it.
//!
//! A path is resolved before it is checked: made absolute, `.` and `..`
//! taken out, and symlinks followed, so a link planted in a project cannot
//! lead out of it. An extension reads and writes its projects' folders and
//! its own; it reads a file a person handed it; anywhere else needs
//! `process.run`, which can reach it anyway. Vorn's own data, the keys in
//! `~/.ssh` and the shell's startup files are never reached.

use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};

use crate::api::ExtError;

/// What a `files.*` call does at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// `files.list` and `files.stat`: names, sizes and times, never contents.
    Look,
    /// `files.read`.
    Read,
    /// `files.write`.
    Write,
}

/// In the home folder, never reached by `files.*`: credentials and what a shell runs as it starts.
const CLOSED_IN_HOME: &[&str] = &[
    ".vorn",
    ".ssh",
    ".gnupg",
    ".aws",
    ".kube",
    ".docker",
    ".netrc",
    ".config/gh",
    ".config/fish",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".profile",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".zlogout",
];

/// The resolved places one extension's `files.*` calls are held to.
#[derive(Debug, Default, Clone)]
pub struct Places {
    /// Every project's folder and its worktrees': read and written.
    pub roots: Vec<PathBuf>,
    /// The folders a session's shell is in: the files directly in them may be read.
    pub here: Vec<PathBuf>,
    /// The extension's own folder of files.
    pub own: Option<PathBuf>,
    /// What a person handed the extension: read.
    pub handed: Vec<PathBuf>,
    /// Never reached, not even with `process.run`.
    pub closed: Vec<PathBuf>,
}

/// The places never reached under `home`, with Vorn's `data` folders, each resolved.
pub fn closed(home: &Path, data: &[&Path]) -> Vec<PathBuf> {
    CLOSED_IN_HOME
        .iter()
        .map(|p| home.join(p))
        .chain(data.iter().map(|d| d.to_path_buf()))
        .filter_map(|p| resolve(&p).ok())
        .collect()
}

/// `path` as the file system names it: absolute, `.` and `..` gone and
/// symlinks followed. A path not there yet is resolved through the
/// nearest folder above it that is.
pub fn resolve(path: &Path) -> Result<PathBuf, ExtError> {
    if !path.is_absolute() {
        return Err(ExtError::new(
            "bad-request",
            format!("{} is not an absolute path", path.display()),
        ));
    }
    let mut missing = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            let mut out = real;
            for part in missing.iter().rev() {
                match part {
                    Component::Normal(name) => out.push(name),
                    Component::ParentDir => {
                        out.pop();
                    }
                    _ => {}
                }
            }
            return Ok(out);
        }
        let Some(last) = at.components().next_back() else {
            return Err(ExtError::new(
                "bad-request",
                format!("{} does not resolve", path.display()),
            ));
        };
        missing.push(last);
        at = match at.parent() {
            Some(parent) => parent,
            None => {
                return Err(ExtError::new(
                    "bad-request",
                    format!("{} does not resolve", path.display()),
                ))
            }
        };
    }
}

impl Places {
    /// Whether `extension` may `access` the resolved `path`; `runs_commands`
    /// when it declares `process.run`.
    pub fn admit(
        &self,
        extension: &str,
        path: &Path,
        access: Access,
        runs_commands: bool,
    ) -> Result<(), ExtError> {
        if self.own.as_deref().is_some_and(|own| path.starts_with(own)) {
            return Ok(());
        }
        if self.closed.iter().any(|c| path.starts_with(c)) {
            return Err(ExtError::new(
                "denied",
                format!("{} is not open to extensions", path.display()),
            ));
        }
        let inside = |set: &[PathBuf]| set.iter().any(|r| path.starts_with(r));
        let open = match access {
            Access::Look => true,
            Access::Read => {
                inside(&self.roots)
                    || inside(&self.handed)
                    || path
                        .parent()
                        .is_some_and(|p| self.here.iter().any(|h| h == p))
            }
            Access::Write => inside(&self.roots),
        };
        if open || runs_commands {
            return Ok(());
        }
        let does = if access == Access::Write {
            "writes"
        } else {
            "reads"
        };
        Err(ExtError::new(
            "denied",
            format!(
                "{extension} {does} only the files of its projects, and {} is outside them; anywhere else needs process.run",
                path.display()
            ),
        ))
    }
}

/// How many paths a person handed, and sessions started, each extension keeps.
const KEPT: usize = 64;

/// What each extension was handed by a person or started itself, which
/// widens what it reaches, and whether a person is acting through it now.
#[derive(Debug, Default)]
pub struct Given {
    paths: HashMap<String, VecDeque<PathBuf>>,
    sessions: HashMap<String, VecDeque<String>>,
    acting: HashMap<String, usize>,
}

fn keep<T: PartialEq>(list: &mut VecDeque<T>, item: T) {
    if list.contains(&item) {
        return;
    }
    if list.len() == KEPT {
        list.pop_front();
    }
    list.push_back(item);
}

impl Given {
    /// A person chose `path` for `extension`, in a file dialog or a form.
    pub fn hand(&mut self, extension: &str, path: PathBuf) {
        keep(self.paths.entry(extension.to_owned()).or_default(), path);
    }

    /// The paths a person handed `extension`, newest last.
    pub fn handed(&self, extension: &str) -> Vec<PathBuf> {
        self.paths
            .get(extension)
            .map(|p| p.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// `extension` started session `id`.
    pub fn started(&mut self, extension: &str, id: &str) {
        keep(
            self.sessions.entry(extension.to_owned()).or_default(),
            id.to_owned(),
        );
    }

    /// Whether `extension` started session `id`.
    pub fn starter_of(&self, extension: &str, id: &str) -> bool {
        self.sessions
            .get(extension)
            .is_some_and(|s| s.iter().any(|s| s == id))
    }

    /// A person's click or choice is being answered by `extension`.
    pub fn begin(&mut self, extension: &str) {
        *self.acting.entry(extension.to_owned()).or_default() += 1;
    }

    /// The call [`Given::begin`] counted is done.
    pub fn end(&mut self, extension: &str) {
        if let Some(n) = self.acting.get_mut(extension) {
            *n -= 1;
            if *n == 0 {
                self.acting.remove(extension);
            }
        }
    }

    /// Whether `extension` is answering a person's click or choice now.
    pub fn acting(&self, extension: &str) -> bool {
        self.acting.contains_key(extension)
    }

    /// Forgets `extension`, as it is removed.
    pub fn forget(&mut self, extension: &str) {
        self.paths.remove(extension);
        self.sessions.remove(extension);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Disk {
        _dir: tempfile::TempDir,
        root: PathBuf,
        home: PathBuf,
        project: PathBuf,
        own: PathBuf,
    }

    fn disk() -> Disk {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let home = root.join("home");
        let project = home.join("code/app");
        let own = home.join(".vorn/blobs/x");
        for d in [&project, &own, &home.join(".ssh"), &home.join("elsewhere")] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(project.join("a.txt"), "a").unwrap();
        fs::write(home.join(".ssh/id_ed25519"), "key").unwrap();
        fs::write(home.join(".zshrc"), "rc").unwrap();
        fs::write(home.join("elsewhere/b.txt"), "b").unwrap();
        Disk {
            _dir: dir,
            root,
            home,
            project,
            own,
        }
    }

    fn places(d: &Disk) -> Places {
        Places {
            roots: vec![d.project.clone()],
            here: Vec::new(),
            own: Some(d.own.clone()),
            handed: Vec::new(),
            closed: closed(&d.home, &[&d.home.join(".vorn")]),
        }
    }

    fn admit(p: &Places, path: &Path, access: Access, runs: bool) -> Result<(), ExtError> {
        p.admit("x", &resolve(path)?, access, runs)
    }

    #[test]
    fn reaches_its_projects_and_its_own_folder() {
        let d = disk();
        let p = places(&d);
        for access in [Access::Look, Access::Read, Access::Write] {
            assert_eq!(admit(&p, &d.project.join("a.txt"), access, false), Ok(()));
            assert_eq!(admit(&p, &d.own.join("f.png"), access, false), Ok(()));
        }
        assert_eq!(
            admit(&p, &d.project.join("new/deeper.txt"), Access::Write, false),
            Ok(())
        );
    }

    #[test]
    fn reaches_elsewhere_only_to_look_or_with_process_run() {
        let d = disk();
        let p = places(&d);
        let b = d.home.join("elsewhere/b.txt");
        assert_eq!(admit(&p, &b, Access::Look, false), Ok(()));
        for access in [Access::Read, Access::Write] {
            let err = admit(&p, &b, access, false).unwrap_err();
            assert_eq!(err.code, "denied");
            assert!(err.message.contains("process.run"), "{}", err.message);
            assert_eq!(admit(&p, &b, access, true), Ok(()));
        }
    }

    #[test]
    fn never_reaches_vorns_data_keys_or_startup_files() {
        let d = disk();
        let p = places(&d);
        for path in [
            d.home.join(".ssh/id_ed25519"),
            d.home.join(".ssh"),
            d.home.join(".zshrc"),
            d.home.join(".vorn/vorn.db"),
            d.home.join(".vorn/blobs/y/f.png"),
        ] {
            for access in [Access::Look, Access::Read, Access::Write] {
                let err = admit(&p, &path, access, true).unwrap_err();
                assert_eq!(err.code, "denied", "{}", path.display());
            }
        }
    }

    #[test]
    fn resolves_dot_dot_and_symlinks_before_it_checks() {
        let d = disk();
        let p = places(&d);
        let climbs = d.project.join("../../.ssh/id_ed25519");
        assert!(admit(&p, &climbs, Access::Read, true).is_err());
        let climbs_new = d.project.join("../../elsewhere/new.txt");
        assert!(admit(&p, &climbs_new, Access::Write, false).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.home.join(".ssh"), d.project.join("keys")).unwrap();
            std::os::unix::fs::symlink(d.home.join("elsewhere"), d.project.join("out")).unwrap();
            let link = d.project.join("keys/id_ed25519");
            assert!(admit(&p, &link, Access::Read, false).is_err());
            assert!(admit(&p, &d.project.join("out/b.txt"), Access::Read, false).is_err());
            assert!(admit(&p, &d.project.join("out/c.txt"), Access::Write, false).is_err());
        }
        assert_eq!(
            resolve(Path::new("relative")).unwrap_err().code,
            "bad-request"
        );
        assert_eq!(
            resolve(&d.root.join("gone/../home/code/app/a.txt")).unwrap(),
            d.project.join("a.txt")
        );
    }

    #[test]
    fn reads_what_a_person_handed_it_and_beside_a_shell() {
        let d = disk();
        let mut p = places(&d);
        let b = d.home.join("elsewhere/b.txt");
        p.handed = vec![resolve(&b).unwrap()];
        assert_eq!(admit(&p, &b, Access::Read, false), Ok(()));
        assert!(admit(&p, &b, Access::Write, false).is_err());
        assert!(admit(&p, &d.home.join("elsewhere/c.txt"), Access::Read, false).is_err());

        let mut p = places(&d);
        p.here = vec![d.home.clone()];
        let pkg = d.home.join("package.json");
        assert_eq!(admit(&p, &pkg, Access::Read, false), Ok(()));
        assert!(admit(&p, &b, Access::Read, false).is_err());
        assert!(admit(&p, &d.home.join(".zshrc"), Access::Read, false).is_err());
    }

    #[test]
    fn keeps_what_was_handed_and_started_per_extension() {
        let mut g = Given::default();
        g.hand("x", PathBuf::from("/a"));
        g.hand("x", PathBuf::from("/a"));
        g.started("x", "s1");
        assert_eq!(g.handed("x"), [PathBuf::from("/a")]);
        assert!(g.handed("y").is_empty());
        assert!(g.starter_of("x", "s1") && !g.starter_of("y", "s1"));
        for i in 0..KEPT + 1 {
            g.started("x", &format!("n{i}"));
        }
        assert!(!g.starter_of("x", "s1"), "the oldest goes past the cap");
        assert!(g.starter_of("x", &format!("n{KEPT}")));

        assert!(!g.acting("x"));
        g.begin("x");
        g.begin("x");
        g.end("x");
        assert!(g.acting("x") && !g.acting("y"));
        g.end("x");
        assert!(!g.acting("x"));
        g.forget("x");
        assert!(g.handed("x").is_empty());
    }
}

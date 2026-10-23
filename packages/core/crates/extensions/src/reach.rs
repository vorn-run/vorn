//! Where an extension's `files.*` calls reach, and what a person handed it.
//!
//! A path is resolved before it is checked: made absolute, `.` and `..`
//! taken out, and symlinks followed, so a link planted in a project cannot
//! lead out of it; a link to nothing, or a `..` past a folder that is not
//! there, is refused. An extension reads and writes its projects' folders,
//! its own, and the folders its sessions' shells are in; it reads a file a
//! person handed it; anywhere else needs `process.run`. The home folder
//! itself is never a project's or a shell's folder here, and Vorn's own
//! data, credentials and the shell's startup files are never reached.
//!
//! What was checked is then opened without following a link ([`open`]) and
//! used through that handle, never by name again. A file with more than one
//! name (hard links) is never written, since its other names may be
//! anywhere; one is read unless another of its names is in a closed place.

mod open;

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fs::{File, Metadata};
use std::io;
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

/// In the home folder, never reached by `files.*`: credentials, and what
/// runs as a shell or a login starts.
const CLOSED_IN_HOME: &[&str] = &[
    ".vorn",
    ".ssh",
    ".gnupg",
    ".aws",
    ".kube",
    ".docker",
    ".netrc",
    ".git-credentials",
    ".npmrc",
    ".pypirc",
    ".pgpass",
    ".claude.json",
    ".config/gh",
    ".config/gcloud",
    ".azure",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".gem/credentials",
    ".config/fish",
    ".config/autostart",
    ".config/systemd",
    "Library/LaunchAgents",
    "AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup",
    "Documents/PowerShell",
    "Documents/WindowsPowerShell",
    ".bashrc",
    ".bash_aliases",
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
    /// The folders the shells of its sessions are in: read and written; for
    /// the home folder, read below but never written; above it, only the
    /// files directly in that folder read.
    pub here: Vec<PathBuf>,
    /// The home folder, which with every folder above it is never a root.
    pub home: Option<PathBuf>,
    /// The extension's own folder of files.
    pub own: Option<PathBuf>,
    /// What a person handed the extension: read.
    pub handed: Vec<PathBuf>,
    /// Never reached through `files.*`, even with `process.run`.
    pub closed: Vec<PathBuf>,
}

/// The places never reached under `home`, with Vorn's `data` folders, each
/// resolved. A data folder holding `home` is left open, or nothing in the
/// home folder would be reached; `.vorn` in it stays closed.
pub fn closed(home: &Path, data: &[&Path]) -> Vec<PathBuf> {
    let home_real = resolve(home).ok();
    let data = data
        .iter()
        .filter_map(|d| resolve(d).ok())
        .filter(|d| home_real.as_ref().is_none_or(|h| !h.starts_with(d)));
    CLOSED_IN_HOME
        .iter()
        .filter_map(|p| resolve(&home.join(p)).ok())
        .chain(data)
        .collect()
}

/// `path` as the file system names it: absolute, `.` and `..` gone and
/// symlinks followed, one component at a time. A path not there yet keeps
/// its missing tail as named, which may not hold a `..`; a link that leads
/// nowhere is refused, since what it names could be made later.
pub fn resolve(path: &Path) -> Result<PathBuf, ExtError> {
    let refused = |why: &str| ExtError::new("bad-request", format!("{} {why}", path.display()));
    if !path.is_absolute() {
        return Err(refused("is not an absolute path"));
    }
    let anchor: PathBuf = path
        .components()
        .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
        .collect();
    let mut out = std::fs::canonicalize(&anchor).unwrap_or(anchor);
    let mut missing = false;
    for part in path.components() {
        match part {
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
            Component::ParentDir if missing => {
                return Err(refused("goes up from a folder that is not there"));
            }
            // Windows takes `..` out by name before it follows a link or junction,
            // so the path it opens may not be the one checked here.
            Component::ParentDir if cfg!(windows) => {
                return Err(refused("goes up with .., which Windows reads before links"));
            }
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => {
                out.push(name);
                if missing {
                    continue;
                }
                // Canonical names each folder as the disk does, case included, with links followed.
                match std::fs::canonicalize(&out) {
                    Ok(real) => out = real,
                    Err(_) => match std::fs::symlink_metadata(&out) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing = true,
                        Ok(m) if m.file_type().is_symlink() => {
                            return Err(refused("leads through a link to nothing"));
                        }
                        _ => return Err(refused("does not resolve")),
                    },
                }
            }
        }
    }
    Ok(out)
}

/// Whether the resolved `path` is in a git folder: one named `.git`, or one
/// git would read as a repository's (a `HEAD` beside `objects` or a
/// `commondir`), which a worktree's `.git` file or a bare clone points to.
fn in_git(path: &Path) -> bool {
    let named_git = |c: Component| match c {
        Component::Normal(n) => n.to_string_lossy().eq_ignore_ascii_case(".git"),
        _ => false,
    };
    path.components().any(named_git)
        || path.ancestors().any(|dir| {
            dir.join("HEAD").is_file()
                && (dir.join("objects").is_dir() || dir.join("commondir").is_file())
        })
}

/// Whether `path` is `dir` or inside it, by name as the disk compares
/// names: without case on macOS and Windows.
fn within(path: &Path, dir: &Path) -> bool {
    if cfg!(any(target_os = "macos", windows)) {
        let lower = |p: &Path| p.to_string_lossy().to_lowercase();
        Path::new(&lower(path)).starts_with(lower(dir))
    } else {
        path.starts_with(dir)
    }
}

impl Places {
    /// `path` as the caller named it, once `extension` may `access` it as
    /// resolved: downstream keeps the caller's form (`/tmp`, no `\\?\`).
    pub fn reach(
        &self,
        extension: &str,
        path: &Path,
        access: Access,
        runs_commands: bool,
    ) -> Result<PathBuf, ExtError> {
        self.admit(extension, &resolve(path)?, access, runs_commands)?;
        Ok(path.to_path_buf())
    }

    /// Whether `extension` may `access` the resolved `path`; `runs_commands`
    /// when it declares `process.run`.
    pub fn admit(
        &self,
        extension: &str,
        path: &Path,
        access: Access,
        runs_commands: bool,
    ) -> Result<(), ExtError> {
        // Git runs what a repository's config names, so writing there runs programs.
        if access == Access::Write && !runs_commands && in_git(path) {
            return Err(ExtError::new(
                "denied",
                format!(
                    "{extension} writes inside a repository's git folder only with process.run, and {} is in one",
                    path.display()
                ),
            ));
        }
        if self.own.as_deref().is_some_and(|own| path.starts_with(own)) {
            return Ok(());
        }
        if self.closed.iter().any(|c| within(path, c)) {
            return Err(ExtError::new(
                "denied",
                format!("{} is not open to extensions", path.display()),
            ));
        }
        // The home folder, or one above it, holds everything: it is never a root.
        let holds_home = |dir: &Path| {
            dir.parent().is_none() || self.home.as_deref().is_some_and(|h| h.starts_with(dir))
        };
        let inside = |set: &[PathBuf]| set.iter().any(|r| !holds_home(r) && path.starts_with(r));
        // A shell in the home folder still shows what is below it; above home, only the files directly in it.
        let above_home = |dir: &Path| holds_home(dir) && self.home.as_deref() != Some(dir);
        let reads_here = self.here.iter().any(|h| {
            if above_home(h) {
                path.parent() == Some(h.as_path())
            } else {
                path.starts_with(h)
            }
        });
        let open = match access {
            Access::Look => true,
            Access::Read => {
                inside(&self.roots) || reads_here || self.handed.iter().any(|h| h == path)
            }
            Access::Write => inside(&self.roots) || inside(&self.here),
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

/// The most entries of the closed places looked through for another name
/// of a file read; past it, the file is taken to have one there.
const CLOSED_LOOKED: usize = 50_000;

pub use open::Looked;

fn became_link(path: &Path) -> ExtError {
    ExtError::new(
        "denied",
        format!(
            "{} changed into a link after it was checked",
            path.display()
        ),
    )
}

/// A file or folder a `files.*` call reached, open: read, written or listed
/// through this handle, never by its name again.
#[derive(Debug)]
pub struct Opened {
    file: File,
    meta: Metadata,
    real: PathBuf,
}

impl Opened {
    /// The open file or folder.
    pub fn file(&self) -> &File {
        &self.file
    }

    /// What the handle said it was as it was opened.
    pub fn metadata(&self) -> &Metadata {
        &self.meta
    }

    /// The path opened, resolved: links followed, as the disk names it.
    pub fn real(&self) -> &Path {
        &self.real
    }

    /// The entries of an open folder, each name with whether it is a folder
    /// (a link to one is not), read through the handle.
    pub fn entries(&self) -> io::Result<Vec<(OsString, bool)>> {
        open::entries(&self.file, &self.real)
    }
}

impl Places {
    /// Opens `path` for `access` once `extension` may reach it ([`Places::reach`]),
    /// through the very path checked: a link swapped in since is refused.
    /// A write creates the file and empties it. The outer error is a
    /// refusal; the inner one, what the disk said.
    pub fn open(
        &self,
        extension: &str,
        path: &Path,
        access: Access,
        runs_commands: bool,
    ) -> Result<io::Result<Opened>, ExtError> {
        let real = resolve(path)?;
        self.admit(extension, &real, access, runs_commands)?;
        let file = match open::open(&real, access) {
            Ok(file) => file,
            Err(open::Failed::Link) => return Err(became_link(path)),
            Err(open::Failed::Io(e)) => return Ok(Err(e)),
        };
        let meta = match file.metadata() {
            Ok(meta) => meta,
            Err(e) => return Ok(Err(e)),
        };
        if access == Access::Write && !meta.is_file() {
            return Ok(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a file",
            )));
        }
        if access != Access::Look && meta.is_file() {
            let names = match open::names(&meta, &file) {
                Ok(n) => n,
                Err(e) => return Ok(Err(e)),
            };
            if names > 1 && access == Access::Write {
                return Err(ExtError::new(
                    "denied",
                    format!(
                        "{} has other names (hard links), which may be anywhere; extensions write only files with one name",
                        path.display()
                    ),
                ));
            }
            if names > 1 {
                match open::identity(&meta, &file) {
                    Ok(id) if !self.closed_holds(id) => {}
                    Ok(_) => {
                        return Err(ExtError::new(
                            "denied",
                            format!(
                                "{} is another name of a file that is not open to extensions",
                                path.display()
                            ),
                        ))
                    }
                    Err(e) => return Ok(Err(e)),
                }
            }
        }
        if access == Access::Write {
            if let Err(e) = file.set_len(0) {
                return Ok(Err(e));
            }
        }
        Ok(Ok(Opened { file, meta, real }))
    }

    /// Looks at `path` once `extension` may reach it ([`Places::reach`]),
    /// in its folder opened as [`Places::open`] opens one, without opening
    /// the file itself: a file it may not read still tells its size and
    /// time. The outer error is a refusal; the inner one, what the disk said.
    pub fn stat(
        &self,
        extension: &str,
        path: &Path,
        runs_commands: bool,
    ) -> Result<io::Result<Looked>, ExtError> {
        let real = resolve(path)?;
        self.admit(extension, &real, Access::Look, runs_commands)?;
        match open::stat(&real) {
            Ok(looked) => Ok(Ok(looked)),
            Err(open::Failed::Link) => Err(became_link(path)),
            Err(open::Failed::Io(e)) => Ok(Err(e)),
        }
    }

    /// Whether the file `id` names has a name in a closed place, as far as
    /// can be shown: past [`CLOSED_LOOKED`] entries, it is taken to have one.
    fn closed_holds(&self, id: open::Identity) -> bool {
        let mut folders = Vec::new();
        for place in &self.closed {
            let Ok(m) = std::fs::symlink_metadata(place) else {
                continue;
            };
            if m.is_dir() {
                folders.push(place.clone());
            } else if m.is_file() && open::identity_at(place) == Some(id) {
                return true;
            }
        }
        let mut left = CLOSED_LOOKED;
        while let Some(dir) = folders.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                if left == 0 {
                    return true;
                }
                left -= 1;
                // Not followed: an entry's own type, a link's included.
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    folders.push(entry.path());
                } else if kind.is_file() && open::identity_at(&entry.path()) == Some(id) {
                    return true;
                }
            }
        }
        false
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
    shown: HashMap<String, VecDeque<String>>,
    acting: HashMap<String, usize>,
}

/// Puts `item` last, as the newest, dropping the oldest past [`KEPT`].
fn keep<T: PartialEq>(list: &mut VecDeque<T>, item: T) {
    list.retain(|i| *i != item);
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

    /// A client draws a tree of `extension` for session `id`: a pane beside
    /// its terminal, or its shell's log.
    pub fn show(&mut self, extension: &str, id: &str) {
        keep(
            self.shown.entry(extension.to_owned()).or_default(),
            id.to_owned(),
        );
    }

    /// Whether `extension` started session `id` or a client draws it there.
    pub fn attached(&self, extension: &str, id: &str) -> bool {
        self.starter_of(extension, id)
            || self
                .shown
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

    /// Session `id` ended: no extension started or shows it any more.
    pub fn ended(&mut self, id: &str) {
        for list in self.sessions.values_mut().chain(self.shown.values_mut()) {
            list.retain(|s| s != id);
        }
    }

    /// Forgets `extension`, as it is removed.
    pub fn forget(&mut self, extension: &str) {
        self.paths.remove(extension);
        self.sessions.remove(extension);
        self.shown.remove(extension);
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
            home: Some(d.home.clone()),
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
    fn closes_a_data_folder_unless_it_holds_the_home_folder() {
        let d = disk();
        let data = d.root.join("data");
        fs::create_dir_all(&data).unwrap();
        assert!(closed(&d.home, &[&data]).contains(&data));
        for holding in [&d.home, &d.root] {
            let shut = closed(&d.home, &[holding]);
            assert!(!shut.contains(holding));
            assert!(shut.contains(&d.home.join(".vorn")));
        }
    }

    #[test]
    fn writes_in_a_git_folder_only_with_process_run() {
        let d = disk();
        let p = places(&d);
        let git = d.project.join(".git");
        fs::create_dir_all(git.join("worktrees/w")).unwrap();
        fs::write(git.join("config"), "[core]\n").unwrap();
        let worktree = d.project.join("w");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", git.join("worktrees/w").display()),
        )
        .unwrap();
        fs::write(git.join("worktrees/w/commondir"), "../..\n").unwrap();
        let apart = d.project.join("apart");
        fs::create_dir_all(apart.join("objects")).unwrap();
        fs::write(apart.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let linked = d.project.join("linked");
        fs::create_dir_all(&linked).unwrap();
        fs::write(linked.join(".git"), format!("gitdir: {}", apart.display())).unwrap();
        for path in [
            git.join("config"),
            git.join("hooks/pre-commit"),
            worktree.join(".git"),
            git.join("worktrees/w/HEAD"),
            apart.join("hooks/pre-commit"),
            apart.join("config"),
        ] {
            let err = admit(&p, &path, Access::Write, false).unwrap_err();
            assert_eq!(err.code, "denied", "{}", path.display());
            assert!(err.message.contains("process.run"), "{}", err.message);
            assert_eq!(admit(&p, &path, Access::Write, true), Ok(()));
            assert_eq!(admit(&p, &path, Access::Read, false), Ok(()));
        }
        for path in [worktree.join("src/a.rs"), d.project.join(".gitignore")] {
            assert_eq!(admit(&p, &path, Access::Write, false), Ok(()));
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
        #[cfg(not(windows))]
        assert_eq!(
            resolve(&d.project.join("../app/./a.txt")).unwrap(),
            d.project.join("a.txt")
        );
        assert_eq!(
            resolve(&d.project.join("new/deeper.txt")).unwrap(),
            d.project.join("new/deeper.txt")
        );
    }

    #[test]
    fn passes_a_path_on_as_the_caller_named_it() {
        let d = disk();
        let p = places(&d);
        let dotted = d.project.join("./a.txt");
        assert_eq!(p.reach("x", &dotted, Access::Read, false), Ok(dotted));
        #[cfg(unix)]
        {
            let alias = d.root.join("alias");
            std::os::unix::fs::symlink(&d.project, &alias).unwrap();
            let named = alias.join("a.txt");
            assert_eq!(p.reach("x", &named, Access::Write, false), Ok(named));
        }
        let outside = d.home.join("elsewhere/b.txt");
        assert!(p.reach("x", &outside, Access::Read, false).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn refuses_any_dot_dot_on_windows() {
        let d = disk();
        let p = places(&d);
        let deep = d.project.join("l/m/n");
        fs::create_dir_all(&deep).unwrap();
        let up = d.project.join(r"l\m\n\..\..\..\a.txt");
        let err = p.reach("x", &up, Access::Write, true).unwrap_err();
        assert_eq!(err.code, "bad-request", "{}", err.message);
        assert_eq!(resolve(&deep.join("..")).unwrap_err().code, "bad-request");
        let named = d.project.join(r"l\m\a.txt");
        assert_eq!(p.reach("x", &named, Access::Write, false), Ok(named));
    }

    #[test]
    fn refuses_going_up_from_a_missing_folder_or_through_a_link_to_nothing() {
        let d = disk();
        let p = places(&d);
        let up = d.root.join("gone/../home/code/app/a.txt");
        assert_eq!(resolve(&up).unwrap_err().code, "bad-request");
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(d.home.join(".ssh"), d.project.join("link")).unwrap();
            let escape = d.project.join("nope/../link/id_ed25519");
            assert!(resolve(&escape).is_err());
            assert!(admit(&p, &escape, Access::Read, false).is_err());

            symlink(d.home.join(".zshenv"), d.project.join("x")).unwrap();
            let err = admit(&p, &d.project.join("x"), Access::Write, true).unwrap_err();
            assert_eq!(err.code, "bad-request", "{}", err.message);
            assert!(resolve(&d.project.join("x/y")).is_err());

            symlink(d.project.join("a.txt"), d.project.join("b.txt")).unwrap();
            assert_eq!(
                admit(&p, &d.project.join("b.txt"), Access::Read, false),
                Ok(())
            );
        }
    }

    #[test]
    fn never_counts_the_home_folder_or_above_it_as_a_project() {
        let d = disk();
        let mut p = places(&d);
        p.roots = vec![
            d.home.clone(),
            d.root.clone(),
            PathBuf::from("/"),
            d.project.clone(),
        ];
        let b = d.home.join("elsewhere/b.txt");
        for access in [Access::Read, Access::Write] {
            assert!(admit(&p, &b, access, false).is_err());
            assert!(admit(&p, &d.home.join("notes.txt"), access, false).is_err());
            assert_eq!(admit(&p, &d.project.join("a.txt"), access, false), Ok(()));
        }
    }

    #[test]
    fn reaches_the_folder_its_sessions_shell_is_in_and_writes_below_home_only() {
        let d = disk();
        let mut p = places(&d);
        p.here = vec![d.home.join("elsewhere")];
        let b = d.home.join("elsewhere/b.txt");
        for access in [Access::Read, Access::Write] {
            assert_eq!(admit(&p, &b, access, false), Ok(()));
            assert_eq!(
                admit(&p, &d.home.join("elsewhere/new/c.txt"), access, false),
                Ok(())
            );
            assert!(admit(&p, &d.home.join("notes.txt"), access, false).is_err());
        }
        p.here = vec![d.home.clone()];
        assert_eq!(
            admit(&p, &d.home.join("notes.txt"), Access::Read, false),
            Ok(())
        );
        assert_eq!(admit(&p, &b, Access::Read, false), Ok(()));
        for path in [d.home.join("notes.txt"), b.clone()] {
            assert!(admit(&p, &path, Access::Write, false).is_err());
        }
        assert!(admit(&p, &d.home.join(".ssh/id_ed25519"), Access::Read, false).is_err());
        p.here = vec![d.root.clone()];
        assert_eq!(
            admit(&p, &d.root.join("top.txt"), Access::Read, false),
            Ok(())
        );
        assert!(admit(&p, &b, Access::Read, false).is_err());
    }

    #[test]
    fn never_reaches_credentials_or_what_runs_at_login() {
        let d = disk();
        let mut p = places(&d);
        p.here = vec![d.home.join("elsewhere")];
        for name in [
            ".git-credentials",
            ".npmrc",
            ".pypirc",
            ".pgpass",
            ".netrc",
            ".docker/config.json",
            ".kube/config",
            ".aws/credentials",
            ".config/gh/hosts.yml",
            ".claude.json",
            "Library/LaunchAgents/run.plist",
            ".config/autostart/run.desktop",
            ".config/systemd/user/run.service",
            ".config/gcloud/credentials.db",
            ".azure/accessTokens.json",
            ".cargo/credentials",
            ".cargo/credentials.toml",
            ".gem/credentials",
            ".bash_aliases",
            ".bash_login",
            ".zlogin",
            ".zprofile",
            ".config/fish/config.fish",
            "AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup/run.bat",
            "Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
            "Documents/WindowsPowerShell/profile.ps1",
        ] {
            let p = Places {
                closed: closed(&d.home, &[]),
                ..p.clone()
            };
            let err = admit(&p, &d.home.join(name), Access::Write, true).unwrap_err();
            assert_eq!(err.code, "denied", "{name}");
        }
        if cfg!(any(target_os = "macos", windows)) {
            let err = admit(&p, &d.home.join(".ZSHENV"), Access::Write, true).unwrap_err();
            assert_eq!(err.code, "denied");
        }
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

        assert!(admit(&p, &d.home.join("elsewhere"), Access::Read, false).is_err());

        let mut p = places(&d);
        p.here = vec![d.home.clone()];
        let pkg = d.home.join("package.json");
        assert_eq!(admit(&p, &pkg, Access::Read, false), Ok(()));
        assert!(admit(&p, &pkg, Access::Write, false).is_err());
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
        g.show("x", "s2");
        assert!(g.attached("x", "s2") && g.attached("x", &format!("n{KEPT}")));
        assert!(!g.attached("y", "s2") && !g.attached("x", "s9"));
        g.forget("x");
        assert!(g.handed("x").is_empty());
        assert!(!g.attached("x", "s2"));
    }

    #[test]
    fn keeps_the_sessions_shown_lately_and_drops_those_that_ended() {
        let mut g = Given::default();
        g.show("x", "s1");
        g.started("x", "s2");
        for i in 0..KEPT - 1 {
            g.show("x", &format!("n{i}"));
        }
        g.show("x", "s1");
        g.show("x", "late");
        assert!(g.attached("x", "s1"), "showing it again keeps it");
        assert!(!g.attached("x", "n0"), "the oldest goes past the cap");
        g.show("y", "s1");
        g.ended("s1");
        g.ended("s2");
        assert!(!g.attached("x", "s1") && !g.attached("y", "s1"));
        assert!(!g.starter_of("x", "s2") && g.attached("x", "late"));
    }

    fn opened(p: &Places, path: &Path, access: Access) -> Result<io::Result<Opened>, ExtError> {
        p.open("x", path, access, false)
    }

    #[test]
    fn opens_what_it_checked_and_reads_writes_and_lists_through_it() {
        use std::io::{Read, Write};
        let d = disk();
        let p = places(&d);
        let mut text = String::new();
        let read = opened(&p, &d.project.join("a.txt"), Access::Read)
            .unwrap()
            .unwrap();
        read.file().read_to_string(&mut text).unwrap();
        assert_eq!(text, "a");

        let new = d.project.join("b.txt");
        let written = opened(&p, &new, Access::Write).unwrap().unwrap();
        written.file().write_all(b"bb").unwrap();
        let again = opened(&p, &new, Access::Write).unwrap().unwrap();
        assert_eq!(again.file().metadata().unwrap().len(), 0, "emptied");
        drop((written, again));

        let mut names = opened(&p, &d.project, Access::Look)
            .unwrap()
            .unwrap()
            .entries()
            .unwrap();
        names.sort();
        assert_eq!(names, [("a.txt".into(), false), ("b.txt".into(), false)]);
        assert!(opened(&p, &d.project.join("gone.txt"), Access::Read)
            .unwrap()
            .is_err());
        assert!(opened(&p, &d.project.join("no/c.txt"), Access::Write)
            .unwrap()
            .is_err());
        assert!(opened(&p, &d.home.join(".zshrc"), Access::Read).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_link_swapped_in_after_the_check() {
        use std::os::unix::fs::symlink;
        let d = disk();
        let outside = d.home.join("elsewhere");
        let sub = d.project.join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("c.txt"), "c").unwrap();
        let checked_folder = resolve(&sub.join("c.txt")).unwrap();
        let checked_file = resolve(&d.project.join("a.txt")).unwrap();

        fs::remove_dir_all(&sub).unwrap();
        symlink(&outside, &sub).unwrap();
        fs::write(outside.join("c.txt"), "secret").unwrap();
        fs::remove_file(d.project.join("a.txt")).unwrap();
        symlink(outside.join("b.txt"), d.project.join("a.txt")).unwrap();
        for access in [Access::Look, Access::Read, Access::Write] {
            for checked in [&checked_folder, &checked_file] {
                assert!(
                    matches!(open::open(checked, access), Err(open::Failed::Link)),
                    "{access:?} {}",
                    checked.display()
                );
            }
        }
        assert_eq!(fs::read_to_string(outside.join("b.txt")).unwrap(), "b");
    }

    #[test]
    fn never_writes_a_file_with_other_names_nor_reads_one_named_in_a_closed_place() {
        let d = disk();
        let p = places(&d);
        let rc = d.project.join("rc");
        fs::hard_link(d.home.join(".zshrc"), &rc).unwrap();
        let err = opened(&p, &rc, Access::Read).unwrap_err();
        assert!(
            err.message.contains("not open to extensions"),
            "{}",
            err.message
        );
        let err = opened(&p, &rc, Access::Write).unwrap_err();
        assert!(err.message.contains("hard links"), "{}", err.message);
        assert_eq!(fs::read_to_string(d.home.join(".zshrc")).unwrap(), "rc");

        let key = d.project.join("key");
        fs::hard_link(d.home.join(".ssh/id_ed25519"), &key).unwrap();
        assert!(opened(&p, &key, Access::Read).is_err());
        assert!(
            opened(&p, &key, Access::Look).unwrap().is_ok(),
            "a look shows no contents"
        );

        let mine = d.project.join("b-too.txt");
        fs::hard_link(d.home.join("elsewhere/b.txt"), &mine).unwrap();
        assert!(opened(&p, &mine, Access::Read).unwrap().is_ok());
        assert!(opened(&p, &mine, Access::Write).is_err());
    }

    #[test]
    fn looks_at_a_file_without_opening_it() {
        let d = disk();
        let p = places(&d);
        let looked = p
            .stat("x", &d.project.join("a.txt"), false)
            .unwrap()
            .unwrap();
        assert!(looked.is_file);
        assert_eq!(looked.len, 1);
        assert!(looked.modified.is_some());
        let folder = p.stat("x", &d.project, false).unwrap().unwrap();
        assert!(!folder.is_file);
        assert!(p
            .stat("x", &d.project.join("gone"), false)
            .unwrap()
            .is_err());
        assert!(p.stat("x", &d.home.join(".zshrc"), false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn looks_at_a_file_only_its_owner_writes_and_goes_through_a_folder_it_may_only_search() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let d = disk();
        let p = places(&d);
        let only_written = d.project.join("w.txt");
        fs::write(&only_written, "www").unwrap();
        fs::set_permissions(&only_written, fs::Permissions::from_mode(0o200)).unwrap();
        assert_eq!(p.stat("x", &only_written, false).unwrap().unwrap().len, 3);

        let shut = d.project.join("shut");
        fs::create_dir(&shut).unwrap();
        fs::write(shut.join("in.txt"), "in").unwrap();
        fs::set_permissions(&shut, fs::Permissions::from_mode(0o100)).unwrap();
        let read = opened(&p, &shut.join("in.txt"), Access::Read).unwrap();
        let looked = p.stat("x", &shut.join("in.txt"), false).unwrap();
        fs::set_permissions(&shut, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(read.is_ok(), "{read:?}");
        assert_eq!(looked.unwrap().len, 2);

        let checked = resolve(&d.project.join("a.txt")).unwrap();
        fs::remove_file(&checked).unwrap();
        symlink(d.home.join("elsewhere/b.txt"), &checked).unwrap();
        assert!(matches!(open::stat(&checked), Err(open::Failed::Link)));
    }
}

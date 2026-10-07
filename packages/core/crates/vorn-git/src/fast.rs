//! The commands gix answers in-process, byte-for-byte as git prints them.
//!
//! Each answer here is a promise that git would print exactly this, so the
//! rule is to decline (return `None`, and git runs) whenever there is any doubt:
//! an environment variable that changes how git finds the repository, a
//! repository this user does not own, a configured `core.worktree`, a ref
//! that `--abbrev-ref` would have to disambiguate, and every failure. git
//! words its own errors, so a command that would fail is always git's to run.
//!
//! Unix only for now: on Windows git prints paths with forward slashes and a
//! drive letter, which these answers have not been checked against.

use std::path::{Path, PathBuf};

use crate::Request;

/// Environment variables that change which repository git finds, or how it
/// reads it. With any of them set, git decides.
const STEERING_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_NAMESPACE",
    "GIT_CONFIG",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_REPLACE_REF_BASE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_SHALLOW_FILE",
];

pub(crate) fn available() -> bool {
    cfg!(unix)
        && !STEERING_ENV
            .iter()
            .any(|name| std::env::var_os(name).is_some())
}

pub(crate) fn answer(req: &Request) -> Option<String> {
    if !available() || steered(req) || !plain_git(req) {
        return None;
    }
    let args: Vec<&str> = req.args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["rev-parse", "--is-inside-work-tree"] => is_inside_work_tree(&req.cwd),
        ["rev-parse", "--show-toplevel"] => show_toplevel(&req.cwd),
        ["rev-parse", "--absolute-git-dir"] => absolute_git_dir(&req.cwd),
        ["rev-parse", "HEAD"] => head_id(&req.cwd),
        ["rev-parse", "--abbrev-ref", "HEAD"] => abbrev_head(&req.cwd),
        _ => None,
    }
}

/// The environment git would get. gix reads the process's, which `available` checked.
fn steered(req: &Request) -> bool {
    STEERING_ENV
        .iter()
        .any(|name| req.env.iter().any(|(key, _)| key == name))
}

/// Whether `req.bin` is git itself rather than something standing in for it.
///
/// An answer from gix is what git prints, so it is only a stand-in for a git
/// binary. A wrapper script on PATH (a shim, a logging or sandboxing wrapper)
/// may print something else or refuse, and then it is the wrapper's to run. A
/// script is recognised by its `#!`; a compiled wrapper named `git` cannot be
/// told apart from git, and is taken at its name.
fn plain_git(req: &Request) -> bool {
    use std::io::Read;
    let bin = Path::new(&req.bin);
    if bin.file_name().and_then(|n| n.to_str()) != Some("git") {
        return false;
    }
    let Some(path) = locate(bin, req) else {
        return false;
    };
    let mut magic = [0u8; 2];
    match std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)) {
        Ok(()) => &magic != b"#!",
        Err(_) => false,
    }
}

/// Where `bin` resolves, as the child process would find it: as given when it
/// is a path, else the first match on the request's PATH.
fn locate(bin: &Path, req: &Request) -> Option<PathBuf> {
    if bin.components().count() > 1 {
        return Some(bin.to_path_buf());
    }
    let path = req
        .env
        .iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.as_str())?;
    std::env::split_paths(path)
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.is_file())
}

/// The repository `cwd` is in, when gix can be trusted to see it as git does.
fn open(cwd: &Path) -> Option<gix::Repository> {
    let repo = gix::discover(cwd).ok()?;
    // git refuses a repository another user owns unless `safe.directory` says
    // otherwise; gix opens it with less trust instead. git decides.
    if repo.git_dir_trust() != gix::sec::Trust::Full {
        return None;
    }
    let config = repo.config_snapshot();
    // A configured worktree or bareness moves or removes the worktree git
    // reports, and a reftable store or another object format is a repository
    // gix may read differently.
    if config.string("core.worktree").is_some()
        || config.boolean("core.bare") == Some(true)
        || config.string("extensions.refStorage").is_some()
        || config.string("extensions.objectFormat").is_some()
        || config.string("extensions.worktreeConfig").is_some()
    {
        return None;
    }
    // Asked from inside the git directory, git answers about that, not the worktree.
    let real_cwd = cwd.canonicalize().ok()?;
    if real_cwd.starts_with(repo.git_dir().canonicalize().ok()?)
        || real_cwd.starts_with(repo.common_dir().canonicalize().ok()?)
    {
        return None;
    }
    Some(repo)
}

fn line(text: impl AsRef<str>) -> String {
    format!("{}\n", text.as_ref())
}

/// git prints these paths resolved, as `realpath` would.
fn real(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

fn path_line(path: &Path) -> Option<String> {
    Some(line(real(path)?.to_str()?))
}

fn is_inside_work_tree(cwd: &Path) -> Option<String> {
    let repo = open(cwd)?;
    repo.workdir()?;
    Some(line("true"))
}

fn show_toplevel(cwd: &Path) -> Option<String> {
    path_line(open(cwd)?.workdir()?)
}

fn absolute_git_dir(cwd: &Path) -> Option<String> {
    path_line(open(cwd)?.git_dir())
}

fn head_id(cwd: &Path) -> Option<String> {
    let repo = open(cwd)?;
    let id = repo.head_id().ok()?;
    Some(line(id.to_string()))
}

fn abbrev_head(cwd: &Path) -> Option<String> {
    let repo = open(cwd)?;
    let head = repo.head().ok()?;
    if head.is_unborn() {
        return None;
    }
    let Some(name) = head.referent_name() else {
        // Detached: git prints HEAD itself.
        return Some(line("HEAD"));
    };
    let full = name.as_bstr().to_str().ok()?;
    let short = full.strip_prefix("refs/heads/")?;
    // `--abbrev-ref` shortens only as far as stays unambiguous, checking the
    // names `rev-parse` tries in order. Any other ref by this name, or a file
    // in the git directory called it, and git prints a longer form.
    let ambiguous = [
        format!("refs/{short}"),
        format!("refs/tags/{short}"),
        format!("refs/remotes/{short}"),
        format!("refs/remotes/{short}/HEAD"),
    ]
    .iter()
    .any(|candidate| !matches!(repo.try_find_reference(candidate.as_str()), Ok(None)));
    if ambiguous || repo.git_dir().join(short).exists() || repo.common_dir().join(short).exists() {
        return None;
    }
    Some(line(short))
}

/// Whether a branch named `name` could be made in `cwd` now, by renaming
/// the one checked out or starting one at a detached HEAD: a valid branch
/// name that no other branch holds or nests with. `None` when gix declines.
pub(crate) fn branch_name_free(cwd: &Path, name: &str) -> Option<bool> {
    if !available() {
        return None;
    }
    let repo = open(cwd)?;
    let head = repo.head().ok()?;
    if head.is_unborn() {
        return None;
    }
    let full = format!("refs/heads/{name}");
    if name == "HEAD" || gix::refs::FullName::try_from(full.as_str()).is_err() {
        return Some(false);
    }
    let current = head.referent_name().map(|n| n.as_bstr().to_owned());
    let branches = repo.references().ok()?;
    for branch in branches.local_branches().ok()? {
        let branch = branch.ok()?;
        let other = branch.name().as_bstr();
        if current.as_ref().is_some_and(|c| c == other) {
            continue;
        }
        let other = other.to_str().ok()?;
        let nests = |a: &str, b: &str| a.strip_prefix(b).is_some_and(|r| r.starts_with('/'));
        if other == full || nests(other, &full) || nests(&full, other) {
            return Some(false);
        }
    }
    Some(true)
}

trait ToStr {
    fn to_str(&self) -> Result<&str, std::str::Utf8Error>;
}

impl ToStr for gix::bstr::BStr {
    fn to_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self)
    }
}

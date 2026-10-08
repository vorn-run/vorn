//! What the server asks of a repository: branches, worktrees, diffs, commits.
//!
//! Each function is one of the server's `git-utils` calls for a local
//! repository, answering exactly what that call answers, failures included:
//! where it collapses an error to `false`, `[]` or `None`, so does this, and
//! where it lets git's error through, this returns that error worded as Node
//! words it. Every command goes through [`crate::run`], so the reads gix
//! answers byte for byte never start a process.
//!
//! The calls block their thread for as long as git runs. A host calls them
//! from a worker thread, and makes the calls that change a repository take
//! turns per [`repo_key`] as the server does, so a commit's `add` and
//! `commit` never have another change between them.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{run, Error, Request};

/// Stdout past this is an error unless a call asks for more, as for the server.
pub const DEFAULT_MAX_BUFFER: usize = 1024 * 1024;

/// As much diff as a reader can use; past this it is a file to open.
pub const MAX_DIFF_TEXT_BYTES: usize = 500 * 1024;

/// The marker a diff cut at [`MAX_DIFF_TEXT_BYTES`] ends with.
const TRUNCATED_DIFF: &str = "\n\n... diff truncated (too large) ...\n";

/// How to run git: the executable and the whole environment it gets, as the
/// server resolves them.
#[derive(Clone, Debug)]
pub struct Git {
    pub bin: String,
    pub env: Vec<(String, String)>,
}

/// A worktree as `git worktree list` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: String,
    /// `detached` when the worktree has no branch.
    pub branch: String,
    /// The first entry git lists: the project itself.
    pub is_main: bool,
    pub name: String,
}

/// A worktree [`Git::create_worktree`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedWorktree {
    pub worktree_path: String,
    pub branch: String,
    pub name: String,
}

/// What `git branch -d` did to each branch it was given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchDeletion {
    pub deleted: Vec<String>,
    /// Each branch git refused, with its message.
    pub failed: Vec<(String, String)>,
}

/// Lines added and removed, over how many files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub files_changed: u64,
    pub insertions: u64,
    pub deletions: u64,
}

/// How a file changed in a diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Renamed,
    Modified,
}

impl FileStatus {
    pub fn name(self) -> &'static str {
        match self {
            FileStatus::Added => "added",
            FileStatus::Deleted => "deleted",
            FileStatus::Renamed => "renamed",
            FileStatus::Modified => "modified",
        }
    }
}

/// One file's part of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    pub file_path: String,
    pub status: FileStatus,
    pub insertions: u64,
    pub deletions: u64,
    /// The file's section of the diff, from its `diff --git` line.
    pub diff: String,
}

/// A diff split by file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FullDiff {
    pub stat: DiffStat,
    pub files: Vec<FileDiff>,
}

/// What the diff is of: the working tree against HEAD, or one commit
/// against another. The ends are given as the client gave them; a missing
/// one is the word `undefined`, as it is when the server formats it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffTarget {
    WorkingTree,
    Range { from: String, to: String },
}

impl DiffTarget {
    fn arg(&self) -> String {
        match self {
            DiffTarget::WorkingTree => "HEAD".to_owned(),
            DiffTarget::Range { from, to } => format!("{from}..{to}"),
        }
    }
}

/// A change that either happened or did not, with git's reason when not.
/// A worktree moved to a new name, as `renameWorktree` answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovedWorktree {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    Ok,
    Failed(String),
}

impl Git {
    /// `git <args>` in `cwd`, trimmed, as the server's `gitExec` answers.
    fn exec(
        &self,
        args: &[&str],
        cwd: &Path,
        timeout_ms: u64,
        max_buffer: usize,
    ) -> Result<String, Error> {
        let req = Request {
            bin: self.bin.clone(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: cwd.to_path_buf(),
            env: self.env.clone(),
            timeout: Duration::from_millis(timeout_ms),
            max_buffer,
        };
        run(&req).map(|reply| js_trim(&reply.stdout).to_owned())
    }

    fn exec_default(&self, args: &[&str], cwd: &Path, timeout_ms: u64) -> Result<String, Error> {
        self.exec(args, cwd, timeout_ms, DEFAULT_MAX_BUFFER)
    }

    /// Whether `cwd` is inside a working tree.
    pub fn is_git_repo(&self, cwd: &Path) -> bool {
        self.exec_default(&["rev-parse", "--is-inside-work-tree"], cwd, 3000)
            .is_ok_and(|out| out == "true")
    }

    /// The branch checked out in `cwd`, or `None` when detached or unreadable.
    pub fn branch(&self, cwd: &Path) -> Option<String> {
        let out = self
            .exec_default(&["rev-parse", "--abbrev-ref", "HEAD"], cwd, 3000)
            .ok()?;
        branch_or_none(js_trim(&out))
    }

    /// Local branch names; empty when git cannot say.
    pub fn list_branches(&self, cwd: &Path) -> Vec<String> {
        self.exec_default(&["branch", "--format=%(refname:short)"], cwd, 5000)
            .map(|out| parse_lines(&out))
            .unwrap_or_default()
    }

    /// Branches on `origin`, after fetching and pruning; empty when either fails.
    pub fn list_remote_branches(&self, cwd: &Path) -> Vec<String> {
        if self
            .exec_default(&["fetch", "--prune"], cwd, 15_000)
            .is_err()
        {
            return Vec::new();
        }
        self.exec_default(&["branch", "-r", "--format=%(refname:short)"], cwd, 5000)
            .map(|out| parse_remote_branches(&out))
            .unwrap_or_default()
    }

    /// The commit checked out in `cwd`, or `None` when git cannot say
    /// (`getGitHead`).
    pub fn head(&self, cwd: &Path) -> Option<String> {
        self.exec_default(&["rev-parse", "HEAD"], cwd, 3000)
            .ok()
            .filter(|out| !out.is_empty())
    }

    /// Checks out `branch` in `cwd` (`checkoutBranch`), with git's refusal
    /// when it refuses.
    pub fn checkout(&self, cwd: &Path, branch: &str) -> Done {
        match self.exec_default(&["checkout", branch], cwd, 10_000) {
            Ok(_) => Done::Ok,
            Err(err) => Done::Failed(err.to_string()),
        }
    }

    /// Renames the branch checked out in `worktree` to `new_branch`, trimmed,
    /// or starts it there when HEAD is detached (`renameWorktreeBranch`).
    /// False when the name is refused or git fails.
    pub fn rename_branch(&self, worktree: &Path, new_branch: &str) -> bool {
        let Some(name) = branch_rename_name(new_branch) else {
            return false;
        };
        let args = match self.branch(worktree) {
            Some(_) => ["branch", "-m", name],
            None => ["switch", "-c", name],
        };
        self.exec_default(&args, worktree, 10_000).is_ok()
    }

    /// Whether [`Git::rename_branch`] would succeed now, read with gix and
    /// without changing anything. `None` when gix cannot tell as git would.
    pub fn foresee_branch_rename(&self, worktree: &Path, new_branch: &str) -> Option<bool> {
        let Some(name) = branch_rename_name(new_branch) else {
            return Some(false);
        };
        crate::fast::branch_name_free(worktree, name)
    }

    /// Moves a vorn worktree to `<parent>/<new name>-<its id>`
    /// (`renameWorktree`). `None` when the name sanitizes to nothing, the
    /// directory carries no id, the target is the worktree itself or is
    /// taken, or git refuses the move.
    pub fn move_worktree(&self, worktree: &str, new_name: &str) -> Option<MovedWorktree> {
        let target = worktree_move_target(worktree, new_name)?;
        if Path::new(&target.path).exists() {
            return None;
        }
        let args = ["worktree", "move", worktree, target.path.as_str()];
        // Not from the worktree itself: Windows cannot move a process's working directory.
        let cwd = common_git_dir(Path::new(worktree))?;
        self.exec_default(&args, &cwd, 10_000).ok()?;
        Some(target)
    }

    /// What [`Git::move_worktree`] would answer now, read without moving
    /// anything: git moves a linked worktree, whose `.git` is a file.
    pub fn foresee_worktree_move(&self, worktree: &str, new_name: &str) -> Option<MovedWorktree> {
        let target = worktree_move_target(worktree, new_name)?;
        let linked = Path::new(worktree).join(".git").is_file();
        (linked && !Path::new(&target.path).exists()).then_some(target)
    }

    /// Makes a worktree for `branch` at
    /// `<parent>/.vorn-worktrees/<project>/<name>-<id>`. A branch that exists
    /// is checked out there, or, when it is checked out elsewhere already, a
    /// new branch is started from it; any other name is a new branch from
    /// HEAD. git's error, when it refuses, is the call's.
    pub fn create_worktree(
        &self,
        project: &str,
        branch: &str,
        worktree_name: Option<&str>,
    ) -> Result<CreatedWorktree, Error> {
        self.create_worktree_at(project, branch, worktree_name, |_| {})
    }

    /// [`Git::create_worktree`], telling `on_path` the new worktree's path
    /// before anything is made there, so the caller can hold it: a cleanup
    /// running while git adds it must not take the half-made directory for
    /// an orphan.
    pub fn create_worktree_at(
        &self,
        project: &str,
        branch: &str,
        worktree_name: Option<&str>,
        on_path: impl FnOnce(&str),
    ) -> Result<CreatedWorktree, Error> {
        let short_id = short_id();
        let raw = match worktree_name {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => generate_name(),
        };
        let name = sanitize_name(&raw);
        let base_dir = worktree_base_dir(project);
        let worktree_dir = format!("{base_dir}{SEP}{name}-{short_id}");
        on_path(&worktree_dir);
        std::fs::create_dir_all(&base_dir).map_err(|error| Error::Fs {
            syscall: "mkdir",
            path: base_dir.clone(),
            error,
        })?;
        let project_dir = Path::new(project);
        let locals = self.list_branches(project_dir);
        let local = |b: &str| locals.iter().any(|l| l == b);
        let created = |branch: String| CreatedWorktree {
            worktree_path: worktree_dir.clone(),
            branch,
            name: name.clone(),
        };
        if local(branch) {
            let add = ["worktree", "add", worktree_dir.as_str(), branch];
            if self.exec_default(&add, project_dir, 30_000).is_err() {
                let new_branch = if local(&name) {
                    format!("{name}-{short_id}")
                } else {
                    name.clone()
                };
                let add = [
                    "worktree",
                    "add",
                    "-b",
                    new_branch.as_str(),
                    worktree_dir.as_str(),
                    branch,
                ];
                self.exec_default(&add, project_dir, 30_000)?;
                return Ok(created(new_branch));
            }
        } else {
            let add = ["worktree", "add", "-b", branch, worktree_dir.as_str()];
            self.exec_default(&add, project_dir, 30_000)?;
        }
        Ok(created(branch.to_owned()))
    }

    /// Removes a worktree, and with `delete_branch` its branch too when git
    /// agrees the branch is merged: `force` discards uncommitted changes,
    /// never unmerged commits. False when git refuses the removal.
    pub fn remove_worktree(
        &self,
        project: &Path,
        worktree: &str,
        force: bool,
        delete_branch: bool,
    ) -> bool {
        // Read before the removal: afterwards git no longer ties it to a path.
        let branch = if delete_branch {
            self.branch(Path::new(worktree))
        } else {
            None
        };
        let mut args = vec!["worktree", "remove", worktree];
        if force {
            args.push("--force");
        }
        if self.exec_default(&args, project, 10_000).is_err() {
            return false;
        }
        if let Some(branch) = branch {
            self.delete_branches(project, &[branch], false);
        }
        true
    }

    /// Whether the worktree has anything `git status` reports; true when it
    /// cannot tell, so nothing is taken for clean that may not be.
    pub fn is_worktree_dirty(&self, worktree: &Path) -> bool {
        self.exec_default(&["status", "--porcelain"], worktree, 5000)
            .map_or(true, |out| !js_trim(&out).is_empty())
    }

    /// Every worktree of the repository, the main one first; empty when git
    /// cannot say.
    pub fn list_worktrees(&self, project: &Path) -> Vec<WorktreeEntry> {
        self.exec_default(&["worktree", "list", "--porcelain"], project, 5000)
            .map(|out| parse_worktree_list(&out))
            .unwrap_or_default()
    }

    /// Deletes each branch with `-d`, so git refuses an unmerged one, or `-D`
    /// with `force`. Goes on past a refusal and reports both sides.
    pub fn delete_branches(
        &self,
        project: &Path,
        branches: &[String],
        force: bool,
    ) -> BranchDeletion {
        let flag = if force { "-D" } else { "-d" };
        let mut done = BranchDeletion::default();
        for branch in branches {
            match self.exec_default(&["branch", flag, branch], project, 10_000) {
                Ok(_) => done.deleted.push(branch.clone()),
                Err(err) => done.failed.push((branch.clone(), err.to_string())),
            }
        }
        done
    }

    /// Lines added and removed against `target`; `None` when git fails.
    pub fn diff_stat(&self, cwd: &Path, target: &DiffTarget) -> Option<DiffStat> {
        let arg = target.arg();
        let out = self
            .exec_default(&["diff", &arg, "--numstat"], cwd, 10_000)
            .ok()?;
        Some(parse_numstat_totals(&out))
    }

    /// The diff against `target`, split by file, each with its own counts;
    /// `None` when git fails. A diff past [`MAX_DIFF_TEXT_BYTES`] is cut there.
    pub fn diff_full(&self, cwd: &Path, target: &DiffTarget) -> Option<FullDiff> {
        let stat = self.diff_stat(cwd, target)?;
        let arg = target.arg();
        let raw = self
            .exec(&["diff", &arg, "-U3"], cwd, 15_000, MAX_DIFF_TEXT_BYTES * 2)
            .ok()?;
        let raw = truncate_diff(raw);
        let numstat = self
            .exec_default(&["diff", &arg, "--numstat"], cwd, 10_000)
            .ok()?;
        Some(FullDiff {
            stat,
            files: split_diff(&raw, &parse_numstat_files(&numstat)),
        })
    }

    /// Commits, after staging everything with `include_unstaged`.
    pub fn commit(&self, cwd: &Path, message: &str, include_unstaged: bool) -> Done {
        if include_unstaged {
            if let Err(err) = self.exec_default(&["add", "-A"], cwd, 10_000) {
                return Done::Failed(err.to_string());
            }
        }
        match self.exec_default(&["commit", "-m", message], cwd, 15_000) {
            Ok(_) => Done::Ok,
            Err(err) => Done::Failed(err.to_string()),
        }
    }

    /// Pushes the current branch where git is configured to.
    pub fn push(&self, cwd: &Path) -> Done {
        match self.exec_default(&["push"], cwd, 30_000) {
            Ok(_) => Done::Ok,
            Err(err) => Done::Failed(err.to_string()),
        }
    }

    /// The GitHub repository `cwd`'s `origin` points at, as the server's
    /// `detectRepoSlug` reads it: `None` for no repository, no origin, no git
    /// or a remote that is not on GitHub.
    pub fn github_origin(&self, cwd: &Path) -> Option<GitHubRepo> {
        let url = self
            .exec_default(&["remote", "get-url", "origin"], cwd, 3000)
            .ok()?;
        parse_github_remote(&url)
    }

    /// The branch a project's work is measured against: `origin/HEAD`, then
    /// the usual local names, then whatever is checked out.
    pub fn default_branch(&self, project: &Path) -> Option<String> {
        let symref = self
            .exec_default(
                &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
                project,
                5000,
            )
            .unwrap_or_default();
        if !symref.is_empty() {
            return Some(symref.strip_prefix("origin/").unwrap_or(&symref).to_owned());
        }
        let locals = self.list_branches(project);
        ["main", "master", "trunk", "develop"]
            .into_iter()
            .find(|c| locals.iter().any(|b| b == c))
            .map(str::to_owned)
            .or_else(|| self.branch(project))
    }

    /// Branches already contained in `base`; empty when git cannot say.
    pub fn merged_branches(&self, project: &Path, base: &str) -> Vec<String> {
        self.exec_default(
            &["branch", "--merged", base, "--format=%(refname:short)"],
            project,
            10_000,
        )
        .map(|out| parse_lines(&out))
        .unwrap_or_default()
    }

    /// Every local branch as `name\tupstream\tcommitter date`, in one call.
    pub fn branch_refs(&self, project: &Path) -> Vec<String> {
        self.exec_default(
            &[
                "for-each-ref",
                "--format=%(refname:short)%09%(upstream:short)%09%(committerdate:iso-strict)",
                "refs/heads",
            ],
            project,
            10_000,
        )
        .map(|out| {
            out.split('\n')
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
    }

    /// The strict ISO date of `rev`'s last commit, or `None` when unreadable.
    pub fn last_commit_date(&self, cwd: &Path, rev: &str) -> Option<String> {
        self.exec_default(&["log", "-1", "--format=%cI", rev], cwd, 5000)
            .ok()
            .filter(|out| !out.is_empty())
    }

    /// The git directory behind a path: `<repo>/.git/worktrees/<name>` for a
    /// linked worktree. `None` outside a repository.
    pub fn absolute_git_dir(&self, cwd: &Path) -> Option<String> {
        self.exec_default(&["rev-parse", "--absolute-git-dir"], cwd, 5000)
            .ok()
            .filter(|out| !out.is_empty())
    }
}

/// Whether `branch` is one vorn generated for a worktree: an adjective-noun
/// pair from its lists, optionally with the worktree's 8-hex id, so cleanup
/// never proposes deleting a branch a person named.
pub fn is_generated_worktree_branch(branch: &str) -> bool {
    let mut parts = branch.split('-');
    let (Some(adjective), Some(noun)) = (parts.next(), parts.next()) else {
        return false;
    };
    let id_ok = match (parts.next(), parts.next()) {
        (None, _) => true,
        (Some(id), None) => {
            id.len() == 8
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        _ => false,
    };
    id_ok && ADJECTIVES.contains(&adjective) && NOUNS.contains(&noun)
}

/// A repository on GitHub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepo {
    pub owner: String,
    pub repo: String,
}

/// The server's `parseGitHubRemote`: the regular expression
/// `^(?:https?://|ssh://)?(?:[^@/]+@)?github\.com[:/]+([^/]+)/(.+?)(?:\.git)?/?$`,
/// ignoring ASCII case, on the trimmed URL, and then no nested path in the
/// repository. Each alternative is tried in the order the expression's
/// backtracking tries it, and the first that matches is the answer.
pub fn parse_github_remote(url: &str) -> Option<GitHubRepo> {
    let url = js_trim(url);
    if url.is_empty() {
        return None;
    }
    for scheme in ["https://", "http://", "ssh://", ""] {
        let Some(rest) = strip_prefix_ascii_ci(url, scheme) else {
            continue;
        };
        // `(?:[^@/]+@)?`: up to the first `@`, when nothing before it is a
        // `/`, tried before going without.
        let user = rest
            .find('@')
            .filter(|&at| at > 0 && !rest[..at].contains('/'))
            .map(|at| &rest[at + 1..]);
        for host in user.into_iter().chain([rest]) {
            if let Some(found) = after_host(host) {
                return found;
            }
        }
    }
    None
}

/// The match from `github.com` on, when there is one: `Some(None)` for a
/// match the nested-path rule refuses.
fn after_host(s: &str) -> Option<Option<GitHubRepo>> {
    let rest = strip_prefix_ascii_ci(s, "github.com")?;
    // `[:/]+` takes all it can, then gives back one at a time: a `:` given
    // back can start the owner.
    let run = rest.len() - rest.trim_start_matches([':', '/']).len();
    (1..=run).rev().find_map(|k| owner_and_repo(&rest[k..]))
}

/// `([^/]+)/(.+?)(?:\.git)?/?$` at the start of `rest`, then the
/// nested-path rule.
fn owner_and_repo(rest: &str) -> Option<Option<GitHubRepo>> {
    let slash = rest.find('/')?;
    let (owner, rest) = (&rest[..slash], &rest[slash + 1..]);
    if owner.is_empty() {
        return None;
    }
    // `(.+?)` is the shortest run, without line terminators (`.` takes
    // none), that leaves `.git/`, `.git`, `/` or nothing.
    let repo = (1..=rest.len())
        .filter(|&k| rest.is_char_boundary(k))
        .find(|&k| {
            let tail = &rest[k..];
            tail.is_empty()
                || tail == "/"
                || tail.eq_ignore_ascii_case(".git")
                || tail.eq_ignore_ascii_case(".git/")
        })
        .map(|k| &rest[..k])?;
    if repo.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    Some((!repo.contains('/')).then(|| GitHubRepo {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
    }))
}

fn strip_prefix_ascii_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// The separator the server joins worktree paths with on this platform.
const SEP: char = Style::HOST.sep();

/// Where a project's worktrees go: `<parent>/.vorn-worktrees/<project>`.
pub fn worktree_base_dir(project: &str) -> String {
    format!(
        "{}{SEP}.vorn-worktrees{SEP}{}",
        node_dirname(project),
        node_basename(project)
    )
}

/// The key the calls that change a repository take turns under: the git
/// directory its main checkout and every linked worktree share, so a commit
/// in a worktree and that worktree's removal from the project wait for each
/// other. Read from disk rather than asked of git, which would be one more
/// command per change.
pub fn repo_key(cwd: &Path) -> PathBuf {
    let dir = absolute(cwd);
    common_git_dir(&dir)
        .or_else(|| vorn_worktree_project(&dir))
        .unwrap_or(dir)
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `<parent>/<project>` for a worktree made at `<parent>/.vorn-worktrees/<project>/<name>`.
fn vorn_worktree_project(dir: &Path) -> Option<PathBuf> {
    let parts: Vec<_> = dir.components().collect();
    let at = parts
        .iter()
        .rposition(|c| c.as_os_str() == ".vorn-worktrees")?;
    let project = parts.get(at + 1)?;
    let mut out: PathBuf = parts[..at].iter().collect();
    out.push(project);
    Some(out)
}

/// The git directory every worktree of `dir`'s repository shares, from `.git`
/// as git reads it.
fn common_git_dir(dir: &Path) -> Option<PathBuf> {
    for at in dir.ancestors() {
        let dot_git = at.join(".git");
        let Ok(meta) = std::fs::metadata(&dot_git) else {
            continue;
        };
        if meta.is_dir() {
            return Some(dot_git);
        }
        if meta.is_file() {
            // A linked worktree: `gitdir: <common>/worktrees/<name>`, whose
            // `commondir` names the shared directory relative to it.
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let git_dir = at.join(text.trim_start_matches("gitdir:").trim());
            let common = std::fs::read_to_string(git_dir.join("commondir")).ok()?;
            return Some(absolute(&git_dir.join(common.trim())));
        }
    }
    None
}

/// JavaScript's `String.prototype.trim`: whitespace and line terminators.
fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// A detached HEAD is no branch.
fn branch_or_none(raw: &str) -> Option<String> {
    (!raw.is_empty() && raw != "HEAD").then(|| raw.to_owned())
}

/// Non-empty lines, each trimmed.
fn parse_lines(out: &str) -> Vec<String> {
    js_trim(out)
        .split('\n')
        .map(js_trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `origin/x` as `x`, without `HEAD`.
fn parse_remote_branches(out: &str) -> Vec<String> {
    js_trim(out)
        .split('\n')
        .map(|b| {
            let b = js_trim(b);
            b.strip_prefix("origin/").unwrap_or(b)
        })
        .filter(|b| !b.is_empty() && *b != "HEAD")
        .map(str::to_owned)
        .collect()
}

/// The totals of `git diff --numstat`: a binary file (`-`) counts as a file
/// with no lines.
fn parse_numstat_totals(out: &str) -> DiffStat {
    let out = js_trim(out);
    let mut stat = DiffStat::default();
    if out.is_empty() {
        return stat;
    }
    for line in out.split('\n') {
        let mut parts = line.split('\t');
        let added = parts.next().unwrap_or("");
        stat.files_changed += 1;
        if added == "-" {
            continue;
        }
        stat.insertions += js_parse_int(added);
        stat.deletions += js_parse_int(parts.next().unwrap_or(""));
    }
    stat
}

/// Each file's counts from `git diff --numstat`, by path.
fn parse_numstat_files(out: &str) -> Vec<(String, u64, u64)> {
    let out = js_trim(out);
    if out.is_empty() {
        return Vec::new();
    }
    out.split('\n')
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                return None;
            }
            let count = |p: &str| if p == "-" { 0 } else { js_parse_int(p) };
            Some((parts[2..].join("\t"), count(parts[0]), count(parts[1])))
        })
        .collect()
}

/// `parseInt(s, 10) || 0` for a count: leading digits after optional
/// whitespace and sign; anything else, or a negative, is 0.
fn js_parse_int(s: &str) -> u64 {
    let s = s.trim_start();
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let end = digits
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    if negative {
        return 0;
    }
    digits[..end].parse().unwrap_or(0)
}

/// `git worktree list --porcelain`, block by block.
fn parse_worktree_list(out: &str) -> Vec<WorktreeEntry> {
    let out = js_trim(out);
    if out.is_empty() {
        return Vec::new();
    }
    let mut worktrees = Vec::new();
    for block in out.split("\n\n") {
        let lines: Vec<&str> = block.split('\n').collect();
        let Some(path) = lines
            .iter()
            .find_map(|l| l.strip_prefix("worktree "))
            .filter(|p| !p.is_empty())
        else {
            continue;
        };
        let branch = lines
            .iter()
            .find(|l| l.starts_with("branch "))
            .map(|l| l.replacen("branch refs/heads/", "", 1))
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "detached".to_owned());
        worktrees.push(WorktreeEntry {
            path: path.to_owned(),
            branch,
            is_main: worktrees.is_empty(),
            name: extract_worktree_name(path),
        });
    }
    worktrees
}

/// A branch name as `renameWorktreeBranch` takes it: trimmed, and never
/// one git would read as an option.
fn branch_rename_name(raw: &str) -> Option<&str> {
    let name = js_trim(raw);
    (!name.is_empty() && !name.starts_with('-')).then_some(name)
}

/// Where `renameWorktree` moves `worktree` for `new_name`: the name
/// sanitized, runs of `-` collapsed and one stripped from each end, beside
/// the worktree and keeping its `-<8 hex>` id.
pub fn worktree_move_target(worktree: &str, new_name: &str) -> Option<MovedWorktree> {
    move_target(Style::HOST, worktree, new_name)
}

fn move_target(style: Style, worktree: &str, new_name: &str) -> Option<MovedWorktree> {
    let mut name = String::new();
    for c in sanitize_name(js_trim(new_name)).chars() {
        if !(c == '-' && name.ends_with('-')) {
            name.push(c);
        }
    }
    let name = name.strip_prefix('-').unwrap_or(&name);
    let name = name.strip_suffix('-').unwrap_or(name);
    if name.is_empty() {
        return None;
    }
    let base = style.basename(worktree);
    let at = base.len().checked_sub(9)?;
    if base.as_bytes()[at] != b'-' {
        return None;
    }
    let id = base.get(at + 1..)?;
    if !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let path = format!("{}{}{name}-{id}", style.dirname(worktree), style.sep());
    (!style.same_path(&path, worktree)).then(|| MovedWorktree {
        path,
        name: name.to_owned(),
    })
}

/// A worktree's name: its directory without the `-<8 hex>` id vorn adds.
pub fn extract_worktree_name(worktree: &str) -> String {
    let base = node_basename(worktree);
    match base.len().checked_sub(9) {
        Some(at)
            if at > 0
                && base.as_bytes()[at] == b'-'
                && base.as_bytes()[at + 1..]
                    .iter()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                && !base[..at].contains(['\n', '\r', '\u{2028}', '\u{2029}']) =>
        {
            base[..at].to_owned()
        }
        _ => base.to_owned(),
    }
}

/// A worktree name as a directory and branch name: anything but ASCII
/// letters, digits and `-` becomes `-`, one per UTF-16 unit as the server
/// replaces it.
pub fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            out.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                out.push('-');
            }
        }
    }
    out
}

/// Cuts a diff past [`MAX_DIFF_TEXT_BYTES`] UTF-16 units, as the server
/// measures and slices a string, and says so at the end.
fn truncate_diff(raw: String) -> String {
    let mut units = 0usize;
    let mut cut = None;
    for (i, c) in raw.char_indices() {
        let next = units + c.len_utf16();
        if next > MAX_DIFF_TEXT_BYTES {
            cut = Some(i);
            break;
        }
        units = next;
    }
    match cut {
        // A character straddling the limit is left out whole: a Rust string
        // cannot end in half of one, where the server's would.
        Some(at) => {
            let mut out = raw;
            out.truncate(at);
            out.push_str(TRUNCATED_DIFF);
            out
        }
        None => raw,
    }
}

/// Where JavaScript's multiline `^` matches: the start, and after each line
/// terminator.
fn line_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    let after_terminators = text
        .char_indices()
        .filter(|&(_, c)| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
        .map(|(i, c)| i + c.len_utf8());
    std::iter::once(0).chain(after_terminators)
}

/// The rest of the first line that starts with `prefix`, when it is not empty:
/// `/^<prefix>(.+)$/m`.
fn first_line_after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    line_starts(text).find_map(|at| {
        let rest = text[at..].strip_prefix(prefix)?;
        let end = rest
            .find(['\n', '\r', '\u{2028}', '\u{2029}'])
            .unwrap_or(rest.len());
        (end > 0).then(|| &rest[..end])
    })
}

/// The raw diff split at each `diff --git ` that starts a line, each section
/// with its path, status and counts.
fn split_diff(raw: &str, counts: &[(String, u64, u64)]) -> Vec<FileDiff> {
    const HEADER: &str = "diff --git ";
    let mut cuts: Vec<usize> = line_starts(raw)
        .filter(|&at| raw[at..].starts_with(HEADER))
        .collect();
    cuts.push(raw.len());
    let mut sections = Vec::with_capacity(cuts.len());
    // What comes before the first header is a section of its own when it is
    // not empty, as `split` leaves it.
    if cuts[0] > 0 {
        sections.push(&raw[..cuts[0]]);
    }
    for pair in cuts.windows(2) {
        let section = &raw[pair[0] + HEADER.len()..pair[1]];
        if !section.is_empty() {
            sections.push(section);
        }
    }
    sections
        .into_iter()
        .map(|section| {
            let full = format!("{HEADER}{section}");
            let file_path = first_line_after(&full, "+++ b/")
                .map(str::to_owned)
                .or_else(|| {
                    first_line_after(&full, "--- a/")
                        .map(|p| if p == "/dev/null" { "" } else { p })
                        .filter(|p| !p.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "unknown".to_owned());
            let status = if full.contains("--- /dev/null") {
                FileStatus::Added
            } else if full.contains("+++ /dev/null") {
                FileStatus::Deleted
            } else if full.contains("rename from") {
                FileStatus::Renamed
            } else {
                FileStatus::Modified
            };
            // The last count for a path wins, as a map filled in order keeps it.
            let (insertions, deletions) = counts
                .iter()
                .rev()
                .find(|(path, ..)| *path == file_path)
                .map_or((0, 0), |(_, i, d)| (*i, *d));
            FileDiff {
                file_path,
                status,
                insertions,
                deletions,
                diff: full,
            }
        })
        .collect()
}

/// Node's `path.basename` for this platform's separators: the last part,
/// trailing separators ignored.
pub fn node_basename(p: &str) -> &str {
    Style::HOST.basename(p)
}

/// Node's `path.dirname` for this platform's separators.
pub fn node_dirname(p: &str) -> &str {
    Style::HOST.dirname(p)
}

/// How a platform separates a path's parts: `/` on POSIX, either slash on
/// Windows. A value rather than `cfg!`, so both are tested on every host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Posix,
    Windows,
}

impl Style {
    const HOST: Style = if cfg!(windows) {
        Style::Windows
    } else {
        Style::Posix
    };

    /// The separator Node's `path.sep` joins with.
    const fn sep(self) -> char {
        match self {
            Style::Posix => '/',
            Style::Windows => '\\',
        }
    }

    fn is_sep(self, c: char) -> bool {
        c == '/' || (self == Style::Windows && c == '\\')
    }

    fn basename(self, p: &str) -> &str {
        let trimmed = p.trim_end_matches(|c| self.is_sep(c));
        if trimmed.is_empty() {
            return "";
        }
        trimmed.rsplit(|c| self.is_sep(c)).next().unwrap_or(trimmed)
    }

    fn dirname(self, p: &str) -> &str {
        if p.is_empty() {
            return ".";
        }
        let trimmed = p.trim_end_matches(|c| self.is_sep(c));
        if trimmed.is_empty() {
            // Only separators: the root.
            return &p[..1];
        }
        match trimmed.rfind(|c| self.is_sep(c)) {
            None => ".",
            Some(at) => {
                let parent = trimmed[..at].trim_end_matches(|c| self.is_sep(c));
                if parent.is_empty() {
                    &p[..1]
                } else {
                    parent
                }
            }
        }
    }

    /// Whether `a` and `b` spell one path: the same parts from the same
    /// root, whichever separators join them and however many.
    fn same_path(self, a: &str, b: &str) -> bool {
        let rooted = |p: &str| p.starts_with(|c| self.is_sep(c));
        rooted(a) == rooted(b) && self.parts(a).eq(self.parts(b))
    }

    fn parts(self, p: &str) -> impl Iterator<Item = &str> {
        p.split(move |c| self.is_sep(c)).filter(|s| !s.is_empty())
    }
}

/// The first eight hex digits of a random UUID, as the server takes them.
fn short_id() -> String {
    let mut id = uuid::Uuid::new_v4().simple().to_string();
    id.truncate(8);
    id
}

const ADJECTIVES: [&str; 25] = [
    "gilded",
    "marble",
    "ornate",
    "sacred",
    "divine",
    "golden",
    "silver",
    "crimson",
    "ivory",
    "velvet",
    "noble",
    "royal",
    "regal",
    "ancient",
    "baroque",
    "classical",
    "tuscan",
    "florentine",
    "venetian",
    "emerald",
    "amber",
    "obsidian",
    "bronze",
    "sienna",
    "scarlet",
];

const NOUNS: [&str; 25] = [
    "fresco",
    "madrigal",
    "etching",
    "sketch",
    "triptych",
    "inkwell",
    "study",
    "canvas",
    "palette",
    "tableau",
    "vellum",
    "relic",
    "mosaic",
    "statue",
    "chapel",
    "garden",
    "fountain",
    "frieze",
    "archive",
    "folio",
    "portrait",
    "scroll",
    "chronicle",
    "stanza",
    "muse",
];

/// An adjective-noun pair from the server's lists, which its stale-branch
/// cleanup recognises as vorn's own.
fn generate_name() -> String {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let pick = |b: u8, n: usize| usize::from(b) % n;
    format!(
        "{}-{}",
        ADJECTIVES[pick(bytes[0], ADJECTIVES.len())],
        NOUNS[pick(bytes[1], NOUNS.len())]
    )
}

impl fmt::Display for FileStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worktree_moves_beside_itself_under_its_new_name_and_id() {
        let target = |p: &str, n: &str| move_target(Style::Posix, p, n).map(|t| (t.path, t.name));
        let at = |name: &str| format!("/w/{name}");
        assert_eq!(
            target("/w/old-1a2b3c4d", "--a  b--"),
            Some((at("a-b-1a2b3c4d"), "a-b".into()))
        );
        assert_eq!(
            target("/w/old-1a2b3c4d", "é"),
            None,
            "one dash per UTF-16 unit, stripped"
        );
        assert_eq!(
            target("/w/old-1a2b3c4d", "x😀"),
            Some((at("x-1a2b3c4d"), "x".into()))
        );
        assert_eq!(target("/w/old-1a2b3c4d", "old"), None);
        assert_eq!(target("/w//old-1a2b3c4d/", "old"), None);
        assert_eq!(target("/w/old-1A2B3C4D", "new"), None);
        assert_eq!(target("/w/1a2b3c4d", "new"), None);
        assert_eq!(target("/w/old_1a2b3c4d", "new"), None);
        assert_eq!(target("/w/é1a2b3c4", "new"), None);
        assert_eq!(target("/w/old-1a2b3c4d", "  "), None);
    }

    #[test]
    fn a_windows_worktree_moves_beside_itself_whichever_slashes_spell_it() {
        let target = |p: &str, n: &str| move_target(Style::Windows, p, n).map(|t| t.path);
        assert_eq!(
            target(r"C:\w\old-1a2b3c4d", "new").as_deref(),
            Some(r"C:\w\new-1a2b3c4d")
        );
        assert_eq!(
            target("C:/w/old-1a2b3c4d", "new").as_deref(),
            Some(r"C:/w\new-1a2b3c4d")
        );
        for same in [
            r"C:\w\old-1a2b3c4d",
            "C:/w/old-1a2b3c4d",
            r"C:\w/old-1a2b3c4d\",
            "/w/old-1a2b3c4d",
            r"\\host\share\old-1a2b3c4d",
        ] {
            assert_eq!(target(same, "old"), None, "{same}");
        }
    }

    #[test]
    fn a_backslash_is_part_of_a_posix_name() {
        assert_eq!(
            move_target(Style::Posix, r"/w\old-1a2b3c4d", "new").map(|t| t.path),
            Some("//new-1a2b3c4d".into())
        );
        assert!(!Style::Posix.same_path(r"/w\a", "/w/a"));
    }

    #[test]
    fn one_path_is_the_same_whichever_separators_join_it() {
        let win = |a, b| Style::Windows.same_path(a, b);
        assert!(win(r"C:\w\a", "C:/w/a"));
        assert!(win(r"C:\\w\a\", "C:/w//a"));
        assert!(win(r"\w\a", "/w/a"));
        assert!(!win(r"\w\a", "w/a"));
        assert!(!win(r"C:\w\a", r"D:\w\a"));
        assert!(!win(r"C:\w\a", r"C:\w\a\b"));
        let posix = |a, b| Style::Posix.same_path(a, b);
        assert!(posix("/w/a", "/w//a/"));
        assert!(!posix("/w/a", "w/a"));
        assert!(!posix("/w/a", "/w/b"));
    }

    #[test]
    fn the_host_moves_a_worktree_it_spells_natively() {
        let wt: PathBuf = [std::path::MAIN_SEPARATOR_STR, "w", "old-1a2b3c4d"]
            .iter()
            .collect();
        let wt = wt.to_str().unwrap();
        assert_eq!(worktree_move_target(wt, "old"), None);
        let moved = worktree_move_target(wt, "new").map(|t| PathBuf::from(t.path));
        assert_eq!(moved, Some(Path::new(wt).with_file_name("new-1a2b3c4d")));
    }

    #[test]
    fn a_branch_name_is_trimmed_and_never_an_option() {
        assert_eq!(branch_rename_name("  a/b \n"), Some("a/b"));
        assert_eq!(branch_rename_name(" -d"), None);
        assert_eq!(branch_rename_name("\t"), None);
    }

    #[test]
    fn reads_branch_lists_trimmed_and_without_blanks() {
        assert_eq!(
            parse_lines("main\nfeature/foo\ndev\n"),
            ["main", "feature/foo", "dev"]
        );
        assert_eq!(parse_lines("  main  \n  dev  \n"), ["main", "dev"]);
        assert!(parse_lines("").is_empty());
        assert_eq!(
            parse_remote_branches("origin/HEAD\norigin/main\norigin/feat/x\nupstream/y\n"),
            ["main", "feat/x", "upstream/y"]
        );
    }

    #[test]
    fn a_detached_or_empty_head_is_no_branch() {
        assert_eq!(branch_or_none("main").as_deref(), Some("main"));
        assert_eq!(branch_or_none("HEAD"), None);
        assert_eq!(branch_or_none(""), None);
    }

    #[test]
    fn totals_numstat_with_binary_files_counted_but_not_their_lines() {
        assert_eq!(
            parse_numstat_totals("10\t5\tsrc/foo.ts\n3\t1\tsrc/bar.ts\n"),
            DiffStat {
                files_changed: 2,
                insertions: 13,
                deletions: 6
            }
        );
        assert_eq!(
            parse_numstat_totals("-\t-\timage.png\n5\t2\tsrc/foo.ts\n"),
            DiffStat {
                files_changed: 2,
                insertions: 5,
                deletions: 2
            }
        );
        assert_eq!(parse_numstat_totals(""), DiffStat::default());
    }

    #[test]
    fn parses_counts_as_parse_int_does() {
        assert_eq!(js_parse_int("12"), 12);
        assert_eq!(js_parse_int("  7x"), 7);
        assert_eq!(js_parse_int("x7"), 0);
        assert_eq!(js_parse_int(""), 0);
        assert_eq!(js_parse_int("-3"), 0);
        assert_eq!(js_parse_int("+4"), 4);
    }

    #[test]
    fn reads_worktree_porcelain_with_the_main_one_first() {
        let out = "worktree /path/to/project\nHEAD abc\nbranch refs/heads/main\n\n\
                   worktree /path/to/feature-1a2b3c4d\nHEAD def\nbranch refs/heads/feature\n\n\
                   worktree /path/to/loose\nHEAD 123\ndetached\n";
        let list = parse_worktree_list(out);
        assert_eq!(
            list,
            [
                WorktreeEntry {
                    path: "/path/to/project".into(),
                    branch: "main".into(),
                    is_main: true,
                    name: "project".into()
                },
                WorktreeEntry {
                    path: "/path/to/feature-1a2b3c4d".into(),
                    branch: "feature".into(),
                    is_main: false,
                    name: "feature".into()
                },
                WorktreeEntry {
                    path: "/path/to/loose".into(),
                    branch: "detached".into(),
                    is_main: false,
                    name: "loose".into()
                },
            ]
        );
        assert!(parse_worktree_list("").is_empty());
    }

    #[test]
    fn names_a_worktree_without_its_id() {
        assert_eq!(
            extract_worktree_name("/a/vivid-nova-abcd1234"),
            "vivid-nova"
        );
        assert_eq!(
            extract_worktree_name("/a/vivid-nova-ABCD1234"),
            "vivid-nova-ABCD1234"
        );
        assert_eq!(extract_worktree_name("/a/-abcd1234"), "-abcd1234");
        assert_eq!(extract_worktree_name("/a/x-abcd123"), "x-abcd123");
        assert_eq!(extract_worktree_name("/a/project/"), "project");
    }

    #[test]
    fn sanitizes_names_one_dash_per_utf16_unit() {
        assert_eq!(sanitize_name("my cool name!"), "my-cool-name-");
        assert_eq!(sanitize_name("vivid-nova"), "vivid-nova");
        assert_eq!(sanitize_name("é"), "-");
        assert_eq!(sanitize_name("a😀b"), "a--b");
    }

    #[test]
    fn generated_names_are_from_the_lists() {
        for _ in 0..50 {
            let name = generate_name();
            let (adj, noun) = name.split_once('-').unwrap();
            assert!(ADJECTIVES.contains(&adj) && NOUNS.contains(&noun), "{name}");
        }
        for _ in 0..20 {
            let name = generate_name();
            assert!(is_generated_worktree_branch(&name), "{name}");
            assert!(is_generated_worktree_branch(&format!(
                "{name}-{}",
                short_id()
            )));
        }
        for named in [
            "main",
            "feature",
            "gilded-fresco-ABCDEF12",
            "gilded-fresco-1234567",
            "gilded-fresco-12345678-x",
            "gilded-person",
            "Gilded-fresco",
            "gilded--fresco",
        ] {
            assert!(!is_generated_worktree_branch(named), "{named}");
        }
        let id = short_id();
        assert_eq!(id.len(), 8);
        assert!(id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }

    #[test]
    fn paths_split_as_node_splits_them_with_each_platforms_separators() {
        assert_eq!(Style::Windows.basename(r"C:\a\b\"), "b");
        assert_eq!(Style::Windows.basename("C:/a/b"), "b");
        assert_eq!(Style::Windows.dirname(r"C:\a/b"), r"C:\a");
        assert_eq!(Style::Windows.dirname(r"\a"), r"\");
        assert_eq!(Style::Posix.basename(r"/a/b\c"), r"b\c");
        assert_eq!(Style::Posix.dirname(r"/a\b"), "/");
        assert_eq!(node_basename("/a/b"), "b");
        assert_eq!(node_basename("/a/b/"), "b");
        assert_eq!(node_dirname("/a/b"), "/a");
        assert_eq!(node_dirname("/a/b/"), "/a");
        assert_eq!(node_dirname("/a"), "/");
        assert_eq!(node_dirname("a"), ".");
        assert_eq!(node_dirname("/"), "/");
        if cfg!(unix) {
            assert_eq!(worktree_base_dir("/src/app"), "/src/.vorn-worktrees/app");
        }
    }

    #[test]
    fn splits_a_diff_by_file_with_status_and_counts() {
        let raw = "diff --git a/new.ts b/new.ts\nnew file mode 100644\n--- /dev/null\n+++ b/new.ts\n@@ -0,0 +1 @@\n+new\n\
                   diff --git a/gone.ts b/gone.ts\ndeleted file mode 100644\n--- a/gone.ts\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n\
                   diff --git a/old.ts b/moved.ts\nsimilarity index 100%\nrename from old.ts\nrename to moved.ts\n\
                   diff --git a/f.ts b/f.ts\n--- a/f.ts\n+++ b/f.ts\n@@ -1 +1 @@\n-a\n+b";
        let counts = parse_numstat_files("1\t0\tnew.ts\n0\t1\tgone.ts\n1\t1\tf.ts");
        let files = split_diff(raw, &counts);
        let shape: Vec<_> = files
            .iter()
            .map(|f| (f.file_path.as_str(), f.status, f.insertions, f.deletions))
            .collect();
        assert_eq!(
            shape,
            [
                ("new.ts", FileStatus::Added, 1, 0),
                ("gone.ts", FileStatus::Deleted, 0, 1),
                ("unknown", FileStatus::Renamed, 0, 0),
                ("f.ts", FileStatus::Modified, 1, 1),
            ]
        );
        assert!(files[3].diff.starts_with("diff --git a/f.ts"));
        assert!(files[0].diff.ends_with("+new\n"));
        assert!(split_diff("", &[]).is_empty());
        // Text before the first header is a section of its own.
        let odd = split_diff("warning: x\ndiff --git a/f b/f\n+++ b/f\n", &[]);
        assert_eq!(odd.len(), 2);
        assert_eq!(odd[0].diff, "diff --git warning: x\n");
    }

    #[test]
    fn cuts_a_long_diff_in_utf16_units_and_says_so() {
        let short = "a".repeat(10);
        assert_eq!(truncate_diff(short.clone()), short);
        let long = "a".repeat(MAX_DIFF_TEXT_BYTES + 5);
        let cut = truncate_diff(long);
        assert!(cut.ends_with(TRUNCATED_DIFF));
        assert_eq!(cut.len(), MAX_DIFF_TEXT_BYTES + TRUNCATED_DIFF.len());
        // A two-unit character counts twice.
        let wide = "😀".repeat(MAX_DIFF_TEXT_BYTES / 2 + 1);
        let cut = truncate_diff(wide);
        assert_eq!(
            cut.trim_end_matches(TRUNCATED_DIFF).chars().count(),
            MAX_DIFF_TEXT_BYTES / 2
        );
    }

    #[test]
    fn finds_the_project_of_a_vorn_worktree() {
        assert_eq!(
            vorn_worktree_project(Path::new("/src/.vorn-worktrees/app/wt-1")),
            Some(PathBuf::from("/src/app"))
        );
        assert_eq!(
            vorn_worktree_project(Path::new("/src/.vorn-worktrees")),
            None
        );
        assert_eq!(vorn_worktree_project(Path::new("/src/app")), None);
    }

    fn gh(owner: &str, repo: &str) -> Option<GitHubRepo> {
        Some(GitHubRepo {
            owner: owner.into(),
            repo: repo.into(),
        })
    }

    #[test]
    fn reads_github_remotes_as_the_server_does() {
        assert_eq!(
            parse_github_remote("git@github.com:vorn-run/vorn.git"),
            gh("vorn-run", "vorn")
        );
        assert_eq!(
            parse_github_remote("https://github.com/vorn-run/connectors.git"),
            gh("vorn-run", "connectors")
        );
        assert_eq!(
            parse_github_remote("ssh://git@github.com/vorn-run/vorn"),
            gh("vorn-run", "vorn")
        );
        assert_eq!(
            parse_github_remote("  https://github.com/a/b/  "),
            gh("a", "b")
        );
        assert_eq!(
            parse_github_remote("git@github.com:owner/my.repo.git"),
            gh("owner", "my.repo")
        );
        assert_eq!(
            parse_github_remote("HTTPS://GitHub.COM/a/b.GIT"),
            gh("a", "b")
        );
        // `[:/]+` gives a `:` back to the owner when it has to.
        assert_eq!(parse_github_remote("git@github.com::/a.git"), gh(":", "a"));
    }

    #[test]
    fn refuses_what_is_not_a_github_repo_root() {
        for url in [
            "git@gitlab.com:vorn-run/vorn.git",
            "https://bitbucket.org/a/b.git",
            "https://github.com/vorn-run/vorn/tree/main",
            "",
            "   ",
            "github.com",
            "https://github.com/onlyowner",
            "https://github.com/a/b\nc",
        ] {
            assert_eq!(parse_github_remote(url), None, "{url:?}");
        }
    }
}

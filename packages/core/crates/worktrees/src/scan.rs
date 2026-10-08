//! The inventory: every worktree of each project, sized and judged
//! (`scanWorktreeInventory`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use vorn_git::repo::{is_generated_worktree_branch, worktree_base_dir, Git};

use crate::guard::is_real_dir;
use crate::size::Sizes;
use crate::verdict::{verdict, Kind, Verdict, VerdictInput};
use crate::{DEFAULT_ARTIFACT_DIRS, DEFAULT_IDLE_DAYS_THRESHOLD};

const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

/// A configured project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: String,
    /// The hosts it is on, `["local"]` when none are named
    /// (`getProjectHostIds`).
    pub host_ids: Vec<String>,
}

/// The person's retention preferences, defaults filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct Retention {
    pub idle_days_threshold: f64,
    pub artifact_dirs: Vec<String>,
    pub pinned_paths: HashSet<String>,
}

impl Retention {
    /// From `defaults.worktreeRetention`, as the server reads it: an empty
    /// list of directories is the default list.
    pub fn from_config(config: Option<&Value>) -> Retention {
        let strings = |key: &str| -> Vec<String> {
            config
                .and_then(|c| c.get(key))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut artifact_dirs = strings("artifactDirs");
        if artifact_dirs.is_empty() {
            artifact_dirs = DEFAULT_ARTIFACT_DIRS
                .iter()
                .map(|d| (*d).to_owned())
                .collect();
        }
        Retention {
            idle_days_threshold: config
                .and_then(|c| c.get("idleDaysThreshold"))
                .and_then(Value::as_f64)
                .unwrap_or(DEFAULT_IDLE_DAYS_THRESHOLD),
            artifact_dirs,
            pinned_paths: strings("pinnedPaths").into_iter().collect(),
        }
    }
}

/// One worktree, or a directory git has forgotten, keys in the server's order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub project_path: String,
    pub project_name: String,
    pub kind: Kind,
    pub branch: Option<String>,
    pub is_main: bool,
    pub size_bytes: u64,
    pub artifact_bytes: u64,
    pub size_measured: bool,
    pub last_commit_at: Option<String>,
    /// The newest git activity in the worktree: its index's mtime.
    pub last_touched_at: Option<String>,
    pub idle_days: Option<u64>,
    pub is_dirty: bool,
    pub is_merged: bool,
    pub has_upstream: bool,
    pub active_session_ids: Vec<String>,
    pub verdict: Verdict,
}

/// A branch left behind by a removed worktree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleBranch {
    pub name: String,
    pub is_merged: bool,
    pub has_upstream: bool,
    pub last_commit_at: Option<String>,
}

/// One project's worktrees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInventory {
    pub project_path: String,
    pub project_name: String,
    pub default_branch: Option<String>,
    /// Always null here: a remote host's projects are the server's to scan.
    pub remote_host_id: Option<String>,
    pub entries: Vec<Entry>,
    pub stale_branches: Vec<StaleBranch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Every project's worktrees, and when they were looked at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub projects: Vec<ProjectInventory>,
    pub scanned_at: String,
}

/// A branch's upstream and last commit date, from `for-each-ref`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchInfo {
    pub upstream: Option<String>,
    pub committer_date: Option<String>,
}

/// What a scan reads with.
pub struct Scan<'a> {
    pub git: &'a Git,
    pub sizes: &'a Sizes,
    pub retention: &'a Retention,
    /// Measure every size again rather than trust the kept ones.
    pub refresh: bool,
    pub now: DateTime<Utc>,
    /// The live sessions running in a worktree, by id.
    pub active: &'a dyn Fn(&str) -> Vec<String>,
}

/// Scans `projects`, only those in `wanted` when it names any. A project on
/// several hosts is scanned once per set of hosts.
pub fn scan(projects: &[Project], wanted: &[String], ctx: &Scan<'_>) -> Inventory {
    let mut seen = HashSet::new();
    let projects = projects
        .iter()
        .filter(|p| wanted.is_empty() || wanted.contains(&p.path))
        .filter(|p| seen.insert(format!("{}:{}", p.host_ids.join(","), p.path)))
        .map(|p| scan_project(p, ctx))
        .collect();
    Inventory {
        projects,
        scanned_at: iso_millis(ctx.now),
    }
}

fn scan_project(project: &Project, ctx: &Scan<'_>) -> ProjectInventory {
    let mut out = ProjectInventory {
        project_path: project.path.clone(),
        project_name: project.name.clone(),
        default_branch: None,
        remote_host_id: None,
        entries: Vec::new(),
        stale_branches: Vec::new(),
        error: None,
    };
    let git = ctx.git;
    let dir = Path::new(&project.path);
    if !git.is_git_repo(dir) {
        out.error = Some("not a git repository".into());
        return out;
    }
    let worktrees = git.list_worktrees(dir);
    if worktrees.is_empty() {
        out.error = Some("could not read worktrees".into());
        return out;
    }
    let default_branch = git.default_branch(dir);
    let branches = branch_info(git, dir);
    let merged: HashSet<String> = default_branch
        .as_deref()
        .map(|base| git.merged_branches(dir, base).into_iter().collect())
        .unwrap_or_default();
    let threshold = ctx.retention.idle_days_threshold;
    let pinned = |p: &str| ctx.retention.pinned_paths.contains(p);

    let mut registered = HashSet::new();
    let mut in_use = HashSet::new();
    for wt in &worktrees {
        registered.insert(wt.path.clone());
        if wt.branch != "detached" {
            in_use.insert(wt.branch.clone());
        }
        let base = Entry {
            path: wt.path.clone(),
            name: wt.name.clone(),
            project_path: project.path.clone(),
            project_name: project.name.clone(),
            kind: Kind::Registered,
            branch: Some(wt.branch.clone()),
            is_main: wt.is_main,
            size_bytes: 0,
            artifact_bytes: 0,
            size_measured: false,
            last_commit_at: None,
            last_touched_at: None,
            idle_days: None,
            is_dirty: false,
            is_merged: false,
            has_upstream: false,
            active_session_ids: Vec::new(),
            verdict: verdict(&main_input(), threshold),
        };
        // The project itself: listed so the totals are honest, never touched.
        if wt.is_main {
            out.entries.push(base);
            continue;
        }
        let size = ctx.sizes.measure(
            &wt.path,
            &ctx.retention.artifact_dirs,
            ctx.refresh,
            &git.env,
        );
        let branch = (wt.branch != "detached").then(|| wt.branch.clone());
        let info = branch.as_ref().and_then(|b| branches.get(b));
        let wt_dir = Path::new(&wt.path);
        let last_commit_at = info
            .and_then(|i| i.committer_date.clone())
            .or_else(|| git.last_commit_date(wt_dir, "HEAD"));
        let last_touched_at = index_mtime(git, wt_dir);
        let active = (ctx.active)(&wt.path);
        let is_dirty = git.is_worktree_dirty(wt_dir);
        let is_merged = branch.as_ref().is_some_and(|b| merged.contains(b));
        let has_upstream = info.is_some_and(|i| i.upstream.is_some());
        let idle_days = days_since(
            newest(last_commit_at.as_deref(), last_touched_at.as_deref()),
            ctx.now,
        );
        let input = VerdictInput {
            is_main: false,
            kind: Kind::Registered,
            is_dirty,
            is_merged,
            has_upstream,
            active_sessions: active.len(),
            is_pinned: pinned(&wt.path),
            size_bytes: size.size_bytes,
            artifact_bytes: size.artifact_bytes,
            idle_days,
        };
        out.entries.push(Entry {
            branch,
            is_main: false,
            size_bytes: size.size_bytes,
            artifact_bytes: size.artifact_bytes,
            size_measured: size.measured,
            last_commit_at,
            last_touched_at,
            idle_days,
            is_dirty,
            is_merged,
            has_upstream,
            active_session_ids: active,
            verdict: verdict(&input, threshold),
            ..base
        });
    }

    for orphan in list_orphan_dirs(&project.path, &registered) {
        let size = ctx
            .sizes
            .measure(&orphan, &ctx.retention.artifact_dirs, ctx.refresh, &git.env);
        let active = (ctx.active)(&orphan);
        let input = VerdictInput {
            is_main: false,
            kind: Kind::OrphanDir,
            is_dirty: false,
            is_merged: false,
            has_upstream: false,
            active_sessions: active.len(),
            is_pinned: pinned(&orphan),
            size_bytes: size.size_bytes,
            artifact_bytes: size.artifact_bytes,
            idle_days: None,
        };
        out.entries.push(Entry {
            name: vorn_git::repo::node_basename(&orphan).to_owned(),
            project_path: project.path.clone(),
            project_name: project.name.clone(),
            kind: Kind::OrphanDir,
            branch: None,
            is_main: false,
            size_bytes: size.size_bytes,
            artifact_bytes: size.artifact_bytes,
            size_measured: size.measured,
            last_commit_at: None,
            last_touched_at: None,
            idle_days: None,
            is_dirty: false,
            is_merged: false,
            has_upstream: false,
            active_session_ids: active,
            verdict: verdict(&input, threshold),
            path: orphan,
        });
    }

    out.stale_branches =
        collect_stale_branches(&branches, &in_use, &merged, default_branch.as_deref());
    out.default_branch = default_branch;
    out
}

fn main_input() -> VerdictInput {
    VerdictInput {
        is_main: true,
        kind: Kind::Registered,
        is_dirty: false,
        is_merged: false,
        has_upstream: false,
        active_sessions: 0,
        is_pinned: false,
        size_bytes: 0,
        artifact_bytes: 0,
        idle_days: None,
    }
}

/// Every local branch's upstream and date, in one `for-each-ref`.
fn branch_info(git: &Git, project: &Path) -> HashMap<String, BranchInfo> {
    git.branch_refs(project)
        .iter()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next().filter(|n| !n.is_empty())?;
            let field = |f: Option<&str>| f.filter(|s| !s.is_empty()).map(str::to_owned);
            let upstream = field(fields.next());
            let committer_date = field(fields.next());
            Some((
                name.to_owned(),
                BranchInfo {
                    upstream,
                    committer_date,
                },
            ))
        })
        .collect()
}

/// When git last wrote the worktree's index, which any git an agent runs
/// does and an unrelated `yarn install` does not.
fn index_mtime(git: &Git, worktree: &Path) -> Option<String> {
    let dir = git.absolute_git_dir(worktree)?;
    let modified = std::fs::metadata(Path::new(&dir).join("index"))
        .and_then(|m| m.modified())
        .ok()?;
    Some(iso_millis(modified.into()))
}

/// `Date.prototype.toISOString`: milliseconds, in UTC.
pub fn iso_millis(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn parse_ms(iso: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// The later of two dates; `b` when they cannot be compared.
fn newest<'a>(a: Option<&'a str>, b: Option<&'a str>) -> Option<&'a str> {
    let (Some(a), Some(b)) = (a, b) else {
        return a.or(b);
    };
    match (parse_ms(a), parse_ms(b)) {
        (Some(x), Some(y)) if x >= y => Some(a),
        _ => Some(b),
    }
}

/// Whole days from `iso` to `now`, never negative.
fn days_since(iso: Option<&str>, now: DateTime<Utc>) -> Option<u64> {
    let then = parse_ms(iso?)?;
    let days = (now.timestamp_millis() - then)
        .div_euclid(MS_PER_DAY)
        .max(0);
    u64::try_from(days).ok()
}

/// Directories under `.vorn-worktrees/<project>` git no longer tracks.
/// `registered` is compared both as written and resolved, since git reports
/// resolved paths and a parent such as `/tmp` may be a symlink. Compared as
/// paths, not strings: on Windows git writes `C:/a/b` and vorn `C:\a\b`.
pub fn list_orphan_dirs(project: &str, registered: &HashSet<String>) -> Vec<String> {
    let known: HashSet<PathBuf> = registered
        .iter()
        .flat_map(|p| [Some(PathBuf::from(p)), std::fs::canonicalize(p).ok()])
        .flatten()
        .collect();
    let base = worktree_base_dir(project);
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    // `readdir` order: by name on the file systems vorn runs on.
    names.sort();
    names
        .into_iter()
        .map(|n| Path::new(&base).join(n))
        .filter(|full| is_real_dir(full))
        .filter(|full| !known.contains(full))
        .filter(|full| std::fs::canonicalize(full).map_or(true, |real| !known.contains(&real)))
        .map(|full| full.to_string_lossy().into_owned())
        .collect()
}

/// Branches left by removed worktrees: vorn's own generated names only, so
/// cleanup never proposes deleting a branch a person named. Sorted by name.
pub fn collect_stale_branches(
    branches: &HashMap<String, BranchInfo>,
    in_use: &HashSet<String>,
    merged: &HashSet<String>,
    default_branch: Option<&str>,
) -> Vec<StaleBranch> {
    let mut stale: Vec<StaleBranch> = branches
        .iter()
        .filter(|(name, _)| Some(name.as_str()) != default_branch)
        .filter(|(name, _)| !in_use.contains(*name))
        .filter(|(name, _)| is_generated_worktree_branch(name))
        .map(|(name, info)| StaleBranch {
            name: name.clone(),
            is_merged: merged.contains(name),
            has_upstream: info.upstream.is_some(),
            last_commit_at: info.committer_date.clone(),
        })
        .collect();
    stale.sort_by(|a, b| a.name.cmp(&b.name));
    stale
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_retention_with_the_defaults_filled_in() {
        let none = Retention::from_config(None);
        assert_eq!(none.idle_days_threshold, 14.0);
        assert_eq!(none.artifact_dirs.len(), DEFAULT_ARTIFACT_DIRS.len());
        assert!(none.pinned_paths.is_empty());
        let empty = Retention::from_config(Some(
            &json!({ "artifactDirs": [], "idleDaysThreshold": null }),
        ));
        assert_eq!(empty, none);
        let set = Retention::from_config(Some(&json!({
            "artifactDirs": ["build"], "idleDaysThreshold": 0, "pinnedPaths": ["/p"],
        })));
        assert_eq!(set.artifact_dirs, vec!["build".to_owned()]);
        assert_eq!(set.idle_days_threshold, 0.0);
        assert!(set.pinned_paths.contains("/p"));
    }

    #[test]
    fn idle_days_count_from_the_newest_activity() {
        let now = DateTime::parse_from_rfc3339("2026-01-21T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let commit = Some("2026-01-01T00:00:00+00:00");
        let touched = Some("2026-01-11T13:00:00.000Z");
        assert_eq!(newest(commit, touched), touched);
        assert_eq!(newest(touched, commit), touched);
        assert_eq!(newest(None, commit), commit);
        assert_eq!(newest(commit, None), commit);
        assert_eq!(newest(Some("garbage"), commit), commit);
        assert_eq!(days_since(touched, now), Some(9));
        assert_eq!(days_since(commit, now), Some(20));
        assert_eq!(days_since(Some("2027-01-01T00:00:00Z"), now), Some(0));
        assert_eq!(days_since(None, now), None);
        assert_eq!(days_since(Some("garbage"), now), None);
        assert_eq!(iso_millis(now), "2026-01-21T12:00:00.000Z");
    }

    #[test]
    fn stale_branches_are_vorns_own_and_unused() {
        let info = |up: Option<&str>| BranchInfo {
            upstream: up.map(str::to_owned),
            committer_date: Some("2026-01-01T00:00:00+00:00".into()),
        };
        let branches: HashMap<String, BranchInfo> = [
            ("main".into(), info(None)),
            ("gilded-fresco".into(), info(Some("origin/gilded-fresco"))),
            ("amber-muse-0123abcd".into(), info(None)),
            ("royal-chapel".into(), info(None)),
            ("my-feature".into(), info(None)),
        ]
        .into();
        let in_use: HashSet<String> = ["royal-chapel".into()].into();
        let merged: HashSet<String> = ["main".into(), "gilded-fresco".into()].into();
        let stale = collect_stale_branches(&branches, &in_use, &merged, Some("main"));
        assert_eq!(
            serde_json::to_value(&stale).unwrap(),
            json!([
                { "name": "amber-muse-0123abcd", "isMerged": false, "hasUpstream": false,
                  "lastCommitAt": "2026-01-01T00:00:00+00:00" },
                { "name": "gilded-fresco", "isMerged": true, "hasUpstream": true,
                  "lastCommitAt": "2026-01-01T00:00:00+00:00" },
            ])
        );
    }

    #[test]
    fn an_orphan_is_a_directory_git_does_not_list() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("proj");
        let base = tmp.path().join(".vorn-worktrees/proj");
        for d in ["kept", "gone", ".hidden"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        std::fs::write(base.join("file"), "x").unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let paths = |v: Vec<String>| v.into_iter().map(PathBuf::from).collect::<Vec<_>>();
        // As git writes it: forward slashes, which on Windows join a path too.
        let git_style = |p: &Path| s(p).replace(std::path::MAIN_SEPARATOR, "/");
        let registered: HashSet<String> = [git_style(&base.join("kept"))].into();
        assert_eq!(
            paths(list_orphan_dirs(&s(&project), &registered)),
            vec![base.join("gone")]
        );
        // Registered under its resolved path only: still not an orphan.
        let real = std::fs::canonicalize(base.join("gone")).unwrap();
        let resolved: HashSet<String> = [s(&base.join("kept")), s(&real)].into();
        assert!(list_orphan_dirs(&s(&project), &resolved).is_empty());
        assert!(list_orphan_dirs(&s(&tmp.path().join("none")), &registered).is_empty());
    }
}

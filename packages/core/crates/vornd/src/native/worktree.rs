//! The worktree manager's calls vornd answers: the inventory, what is safe
//! to remove, and the removal itself, read from its copy of the session
//! records ([`crate::registry`]) and the repositories ([`vorn_worktrees`]).
//!
//! A call is forwarded instead when vornd's copy of the records is not fed
//! yet, when it touches a project on a remote host (the server reaches those
//! over SSH), when vornd cannot read the projects, and when its params are
//! not the shape the server's handler reads. After a change it tells the
//! server which worktrees it cleaned (`vornd:worktreesCleaned`), so the
//! server forgets their sizes as it does after its own.

use std::path::Path;

use serde_json::{json, Value};
use vorn_agents::paths;
use vorn_git::repo::Git;
use vorn_store::{Placement, WorktreeSettings};
use vorn_worktrees::{Cleanup, Guard, Project, RemoveItem, Retention, Scan};

use super::{absolute_str, Answer, Native};
use crate::registry::Registry;

/// The note that names the worktrees a change cleaned.
pub const CLEANED: &str = "vornd:worktreesCleaned";

/// The calls whose shadow answer [`foresee`] predicts: the ones that delete.
const ACTIONS: &[&str] = &[
    "worktree:removeMany",
    "worktree:reclaimArtifacts",
    "worktree:pruneOrphans",
];

/// Answers `method` with `params`.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    if native.registry.get().and_then(|r| r.read(|_| ())).is_none() {
        return Answer::Forward;
    }
    let Some(settings) = settings(native) else {
        return Answer::Forward;
    };
    let git = Git {
        bin: native.env.git_bin(),
        env: native.env.get(),
    };
    match method {
        "worktree:inventory" => inventory(native, &settings, &git, params),
        "git:removeWorktree" => remove_one(native, &settings, &git, params),
        _ => match Asked::read(method, params) {
            Some(asked) if all_local(&settings, &asked.paths()) => {
                act(native, &settings, &git, &asked)
            }
            _ => Answer::Forward,
        },
    }
}

/// Whether [`foresee`] can say what `method` would answer.
pub fn foresees(method: &str) -> bool {
    ACTIONS.contains(&method)
}

/// What vornd would answer a deleting call, for the comparison with the
/// server's answer in shadow mode: its refusal, or an empty result. `None`
/// when the answer is the server's own.
pub fn foresee(native: &Native, method: &str, params: &Value) -> Option<Answer> {
    let asked = Asked::read(method, params)?;
    let settings = settings(native)?;
    if !all_local(&settings, &asked.paths()) {
        return None;
    }
    let refused = native
        .registry
        .get()?
        .read(|r| asked.paths().iter().try_for_each(|p| idle(r, p)))?;
    Some(match refused {
        Ok(()) => Answer::Result(json!({})),
        Err(e) => Answer::Error(e),
    })
}

/// `answer` as shadow mode compares it, for `method`. An inventory without
/// what is measured or the time it was taken, which the two sides take at
/// different moments; a deleting call by whether it was refused up front,
/// since only one side deletes.
pub fn compared(method: &str, mut answer: Value) -> Value {
    if method == "worktree:inventory" {
        let Some(result) = answer.get_mut("result").and_then(Value::as_object_mut) else {
            return answer;
        };
        result.remove("scannedAt");
        let projects = result.get_mut("projects").and_then(Value::as_array_mut);
        for project in projects.into_iter().flatten() {
            let entries = project.get_mut("entries").and_then(Value::as_array_mut);
            for entry in entries
                .into_iter()
                .flatten()
                .filter_map(Value::as_object_mut)
            {
                for key in ["sizeBytes", "artifactBytes", "sizeMeasured"] {
                    entry.remove(key);
                }
                if let Some(verdict) = entry.get_mut("verdict").and_then(Value::as_object_mut) {
                    verdict.remove("freesBytes");
                }
            }
        }
        return answer;
    }
    if foresees(method) {
        return json!({ "refused": answer.pointer("/error/message") });
    }
    answer
}

/// The projects and retention, fresh from the database. `None` when vornd
/// cannot read them.
fn settings(native: &Native) -> Option<WorktreeSettings> {
    match WorktreeSettings::read(native.db.get()?) {
        Ok(settings) => settings,
        Err(err) => {
            tracing::debug!(%err, "could not read the projects; the server answers");
            None
        }
    }
}

/// The projects as the scan reads them, with the hosts the server would
/// read for each.
fn projects(settings: &WorktreeSettings) -> Vec<Project> {
    settings
        .names
        .iter()
        .zip(&settings.hosts.projects)
        .map(|(name, p)| Project {
            name: name.clone(),
            path: p.path.clone(),
            host_ids: match &p.host_ids {
                Some(ids) if !ids.is_empty() => ids.clone(),
                _ => vec!["local".to_owned()],
            },
        })
        .collect()
}

/// The projects on this machine: the ones a local path can belong to.
fn local_projects(settings: &WorktreeSettings) -> Vec<Project> {
    projects(settings)
        .into_iter()
        .zip(&settings.hosts.projects)
        .filter(|(_, p)| settings.hosts.placement(p) == Placement::Local)
        .map(|(p, _)| p)
        .collect()
}

fn all_local(settings: &WorktreeSettings, paths: &[&str]) -> bool {
    paths
        .iter()
        .all(|p| settings.hosts.for_path(p) == Placement::Local)
}

/// `worktree:inventory`: `null`, or `{projectPaths?, refresh?}`.
fn inventory(native: &Native, settings: &WorktreeSettings, git: &Git, params: &Value) -> Answer {
    let (wanted, refresh) = match params {
        Value::Null => (Vec::new(), false),
        Value::Object(_) => {
            let wanted = match params.get("projectPaths") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(list)) => {
                    let Some(list) = list
                        .iter()
                        .map(|p| p.as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()
                    else {
                        return Answer::Forward;
                    };
                    list
                }
                Some(_) => return Answer::Forward,
            };
            let Some(refresh) = flag(params, "refresh") else {
                return Answer::Forward;
            };
            (wanted, refresh)
        }
        _ => return Answer::Forward,
    };
    let in_scope = |p: &&vorn_store::ProjectHost| wanted.is_empty() || wanted.contains(&p.path);
    let remote = settings
        .hosts
        .projects
        .iter()
        .filter(in_scope)
        .any(|p| settings.hosts.placement(p) != Placement::Local);
    if remote {
        return Answer::Forward;
    }
    let Some(registry) = native.registry.get() else {
        return Answer::Forward;
    };
    let active = |path: &str| {
        registry
            .read(|r| {
                r.active_in_worktree(path)
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let retention = Retention::from_config(settings.retention.as_ref());
    let inventory = vorn_worktrees::scan(
        &projects(settings),
        &wanted,
        &Scan {
            git,
            sizes: &native.sizes,
            retention: &retention,
            refresh,
            now: chrono::Utc::now(),
            active: &active,
        },
    );
    Answer::Result(super::json_of(inventory))
}

/// `git:removeWorktree`, which the server answers without asking about
/// sessions: a card closing its own session removes the worktree after it.
fn remove_one(native: &Native, settings: &WorktreeSettings, git: &Git, params: &Value) -> Answer {
    let (Some(project), Some(worktree)) = (
        params.get("projectPath").and_then(absolute_str),
        params.get("worktreePath").and_then(absolute_str),
    ) else {
        return Answer::Forward;
    };
    let (Some(force), Some(delete_branch)) = (flag(params, "force"), flag(params, "deleteBranch"))
    else {
        return Answer::Forward;
    };
    if settings.hosts.for_project(project) != Placement::Local {
        return Answer::Forward;
    }
    native.sizes.invalidate(worktree);
    let removed = native.turns.take(Path::new(project), || {
        git.remove_worktree(Path::new(project), worktree, force, delete_branch)
    });
    tell_cleaned(native, &[worktree.to_owned()]);
    Answer::Result(json!(removed))
}

/// A deleting call's params, as the server's handler reads them.
enum Asked {
    Remove(Vec<RemoveItem>),
    Reclaim(Vec<String>),
    Prune(Vec<String>),
}

impl Asked {
    fn read(method: &str, params: &Value) -> Option<Asked> {
        let paths = || -> Option<Vec<String>> {
            params
                .get("paths")?
                .as_array()?
                .iter()
                .map(|p| absolute_str(p).map(str::to_owned))
                .collect()
        };
        match method {
            "worktree:removeMany" => params
                .get("items")?
                .as_array()?
                .iter()
                .map(|item| {
                    absolute_str(item.get("projectPath")?)?;
                    Some(RemoveItem {
                        worktree_path: absolute_str(item.get("worktreePath")?)?.to_owned(),
                        force: flag(item, "force")?,
                        delete_branch: flag(item, "deleteBranch")?,
                    })
                })
                .collect::<Option<_>>()
                .map(Asked::Remove),
            "worktree:reclaimArtifacts" => paths().map(Asked::Reclaim),
            "worktree:pruneOrphans" => paths().map(Asked::Prune),
            _ => None,
        }
    }

    fn paths(&self) -> Vec<&str> {
        match self {
            Asked::Remove(items) => items.iter().map(|i| i.worktree_path.as_str()).collect(),
            Asked::Reclaim(paths) | Asked::Prune(paths) => {
                paths.iter().map(String::as_str).collect()
            }
        }
    }
}

/// Runs a deleting call once no session is in any of its paths.
fn act(native: &Native, settings: &WorktreeSettings, git: &Git, asked: &Asked) -> Answer {
    let live = Live(native);
    if let Err(refusal) = asked.paths().iter().try_for_each(|p| live.assert_idle(p)) {
        return Answer::Error(refusal);
    }
    let retention = Retention::from_config(settings.retention.as_ref());
    let projects = local_projects(settings);
    let ctx = Cleanup {
        git,
        sizes: &native.sizes,
        artifact_dirs: &retention.artifact_dirs,
        projects: &projects,
        guard: &live,
    };
    let result = match asked {
        Asked::Remove(items) => vorn_worktrees::remove_worktrees(items, &ctx),
        Asked::Reclaim(paths) => vorn_worktrees::reclaim_artifacts(paths, &ctx),
        Asked::Prune(paths) => vorn_worktrees::prune_orphan_dirs(paths, &ctx),
    };
    tell_cleaned(native, &result.succeeded);
    Answer::Result(super::json_of(result))
}

fn tell_cleaned(native: &Native, paths: &[String]) {
    if let (false, Some(link)) = (paths.is_empty(), native.link.get()) {
        link.tell(CLEANED, json!({ "paths": paths }));
    }
}

/// The session records' promise around a deletion: nothing running in the
/// path, nothing starting there, and the repository's turn.
struct Live<'a>(&'a Native);

impl Guard for Live<'_> {
    fn assert_idle(&self, path: &str) -> Result<(), String> {
        self.0
            .registry
            .get()
            .and_then(|r| r.read(|r| idle(r, path)))
            .unwrap_or_else(|| Err(format!("{path} cannot be checked for sessions")))
    }

    fn turn(
        &self,
        project: &Path,
        f: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        self.0.turns.take(project, f)
    }
}

/// The server's `assertIdle`, in its words.
fn idle(r: &Registry, path: &str) -> Result<(), String> {
    let n = r.active_in_worktree(path).len();
    if n > 0 {
        let s = if n > 1 { "s" } else { "" };
        return Err(format!(
            "{path} has {n} active session{s} — close them first"
        ));
    }
    let key = paths::normalize(path);
    if r.holds().contains_key(&key) || r.own_holds().contains_key(&key) {
        return Err(format!("{path} has a session starting — close it first"));
    }
    Ok(())
}

/// An optional boolean field: absent is `false`; anything but a boolean is
/// not a call this answers.
fn flag(params: &Value, key: &str) -> Option<bool> {
    match params.get(key) {
        None | Some(Value::Null) => Some(false),
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Arc;

    use super::*;
    use crate::applink::AppLink;
    use crate::registry::SessionRegistry;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .expect("git runs")
            .success();
        assert!(ok, "git {args:?}");
    }

    /// vornd fed with a repository `p` that has two worktrees: `busy`, which
    /// a running agent uses, and `dirty`, with uncommitted work.
    struct Fixture {
        _dir: tempfile::TempDir,
        native: Arc<Native>,
        registry: Arc<SessionRegistry>,
        link: Arc<AppLink>,
        project: String,
        busy: String,
        dirty: String,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        // Git cannot read Windows' `\\?\` form.
        let root = match root.to_str().and_then(|r| r.strip_prefix(r"\\?\")) {
            Some(plain) => std::path::PathBuf::from(plain),
            None => root,
        };
        let project = root.join("p");
        std::fs::create_dir(&project).unwrap();
        sh(&project, &["init", "-q", "-b", "main"]);
        sh(&project, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let wt = root.join(".vorn-worktrees").join("p");
        let busy = wt.join("busy").to_string_lossy().into_owned();
        let dirty = wt.join("dirty").to_string_lossy().into_owned();
        sh(&project, &["worktree", "add", "-q", "-b", "busy", &busy]);
        sh(&project, &["worktree", "add", "-q", "-b", "dirty", &dirty]);
        std::fs::write(Path::new(&dirty).join("work.txt"), "unsaved").unwrap();

        let db = root.join("vorn.db");
        let (mut store, _) = vorn_store::Store::open(&db, store_options()).unwrap();
        let project = project.to_string_lossy().into_owned();
        store
            .call(
                "dbInsertProject",
                json!([{ "name": "p", "path": project, "preferredAgents": ["claude"] }]),
            )
            .unwrap();
        drop(store);

        let native = Native::new();
        native.set_database(db);
        let registry = SessionRegistry::new();
        native.set_registry(Arc::clone(&registry));
        registry.decide_statuses();
        registry
            .feed(
                1,
                &json!({
                    "op": "snapshot", "headless": [], "order": ["a"],
                    "terminals": [{
                        "id": "a", "agentType": "claude", "projectName": "p",
                        "projectPath": project, "status": "running", "createdAt": 1,
                        "pid": 9, "worktreePath": busy,
                    }],
                }),
            )
            .unwrap();
        let link = Arc::new(AppLink::default());
        native.set_link(Arc::clone(&link));
        Fixture {
            _dir: dir,
            native,
            registry,
            link,
            project,
            busy,
            dirty,
        }
    }

    fn store_options() -> vorn_store::StoreOptions {
        vorn_store::StoreOptions {
            default_shell: "/bin/sh".into(),
            default_agent_commands: serde_json::Map::new(),
            default_workspace: serde_json::from_value(json!({
                "id": "personal", "name": "Personal", "icon": "User",
                "iconColor": "#6b7280", "order": 0,
            }))
            .unwrap(),
            owner_name: "owner".into(),
            seed_workflows: Vec::new(),
        }
    }

    fn entry<'a>(inventory: &'a Value, path: &str) -> &'a Value {
        inventory["projects"][0]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["path"].as_str().map(Path::new) == Some(Path::new(path)))
            .unwrap_or_else(|| panic!("{path} is listed"))
    }

    #[test]
    fn a_worktree_a_live_record_uses_is_kept_and_never_removed() {
        let f = fixture();
        let Answer::Result(inventory) = call(&f.native, "worktree:inventory", &Value::Null) else {
            panic!("vornd answers the inventory");
        };
        let busy = entry(&inventory, &f.busy);
        assert_eq!(busy["activeSessionIds"], json!(["a"]));
        assert_eq!(busy["verdict"]["level"], "keep");

        let items = json!({ "items": [{ "projectPath": f.project, "worktreePath": f.busy, "force": true }] });
        let Answer::Error(refusal) = call(&f.native, "worktree:removeMany", &items) else {
            panic!("a live session refuses the removal");
        };
        assert_eq!(
            refusal,
            format!("{} has 1 active session — close them first", f.busy)
        );
        let paths = json!({ "paths": [f.busy] });
        for method in ["worktree:reclaimArtifacts", "worktree:pruneOrphans"] {
            assert!(matches!(call(&f.native, method, &paths), Answer::Error(_)));
        }
        assert!(Path::new(&f.busy).is_dir());
        assert!(matches!(
            foresee(&f.native, "worktree:removeMany", &items),
            Some(Answer::Error(e)) if e == refusal
        ));
    }

    #[test]
    fn a_worktree_with_uncommitted_work_is_reported_not_removed() {
        let f = fixture();
        let Answer::Result(inventory) = call(&f.native, "worktree:inventory", &Value::Null) else {
            panic!("vornd answers the inventory");
        };
        let dirty = entry(&inventory, &f.dirty);
        assert_eq!(dirty["isDirty"], true);
        assert_eq!(dirty["verdict"]["level"], "review");
        assert_eq!(dirty["verdict"]["reasons"], json!(["uncommitted changes"]));

        let (mut notes, _listening) = f.link.listen();
        let items = json!({ "items": [{ "projectPath": f.project, "worktreePath": f.dirty }] });
        let Answer::Result(result) = call(&f.native, "worktree:removeMany", &items) else {
            panic!("vornd answers the removal");
        };
        assert_eq!(result["succeeded"], json!([]));
        assert_eq!(result["failed"][0]["path"], f.dirty);
        assert!(Path::new(&f.dirty).join("work.txt").is_file());
        assert!(notes.try_recv().is_err(), "nothing was cleaned");
        assert!(matches!(
            foresee(&f.native, "worktree:removeMany", &items),
            Some(Answer::Result(_))
        ));
    }

    #[test]
    fn removes_an_idle_worktree_and_tells_the_server() {
        let f = fixture();
        f.registry
            .feed(
                2,
                &json!({ "op": "snapshot", "headless": [], "order": [], "terminals": [] }),
            )
            .unwrap();
        let (mut notes, _listening) = f.link.listen();
        let items = json!({ "items": [{ "projectPath": f.project, "worktreePath": f.busy, "deleteBranch": true }] });
        let Answer::Result(result) = call(&f.native, "worktree:removeMany", &items) else {
            panic!("vornd answers the removal");
        };
        assert_eq!(result["succeeded"], json!([f.busy]));
        assert_eq!(result["deletedBranches"], json!(["busy"]));
        assert!(!Path::new(&f.busy).exists());
        let note = notes.try_recv().expect("the server is told");
        assert_eq!(note["method"], CLEANED);
        assert_eq!(note["params"]["paths"], json!([f.busy]));

        let one = json!({ "projectPath": f.project, "worktreePath": f.dirty, "force": true });
        assert!(matches!(
            call(&f.native, "git:removeWorktree", &one),
            Answer::Result(Value::Bool(true))
        ));
        assert!(!Path::new(&f.dirty).exists());
    }

    #[test]
    fn a_starting_session_refuses_the_removal() {
        let f = fixture();
        let key = paths::normalize(&f.dirty);
        f.registry.change(|r| ((), vec![r.hold(&key)]));
        let paths = json!({ "paths": [f.dirty] });
        let Answer::Error(refusal) = call(&f.native, "worktree:reclaimArtifacts", &paths) else {
            panic!("a starting session refuses it");
        };
        assert_eq!(
            refusal,
            format!("{} has a session starting — close it first", f.dirty)
        );
    }

    #[test]
    fn forwards_what_the_server_answers() {
        let f = fixture();
        let calls = [
            ("worktree:inventory", json!({ "projectPaths": "no" })),
            ("worktree:inventory", json!({ "refresh": 1 })),
            (
                "worktree:removeMany",
                json!({ "items": [{ "worktreePath": "rel" }] }),
            ),
            ("worktree:pruneOrphans", json!({ "paths": ["rel"] })),
            ("git:removeWorktree", json!({ "projectPath": f.project })),
        ];
        for (method, params) in calls {
            assert!(
                matches!(call(&f.native, method, &params), Answer::Forward),
                "{method} {params}"
            );
        }
        assert!(foresee(&f.native, "worktree:pruneOrphans", &json!({})).is_none());

        let unfed = Native::new();
        unfed.set_registry(SessionRegistry::new());
        assert!(matches!(
            call(&unfed, "worktree:inventory", &Value::Null),
            Answer::Forward
        ));
    }

    #[test]
    fn forwards_a_project_on_a_remote_host() {
        let settings = WorktreeSettings {
            names: vec!["r".into(), "l".into()],
            hosts: vorn_store::ProjectHosts {
                projects: vec![
                    vorn_store::ProjectHost {
                        path: "/r".into(),
                        host_ids: Some(vec!["h".into()]),
                    },
                    vorn_store::ProjectHost {
                        path: "/l".into(),
                        host_ids: None,
                    },
                ],
                remote_hosts: vec!["h".into()],
            },
            retention: None,
        };
        assert!(!all_local(&settings, &["/r/x"]));
        assert!(all_local(&settings, &["/l/x"]));
        assert_eq!(local_projects(&settings)[0].path, "/l");
        assert_eq!(projects(&settings)[1].host_ids, ["local"]);
    }

    #[test]
    fn compares_inventories_without_what_is_measured() {
        let a = json!({ "result": {
            "scannedAt": "t1",
            "projects": [{ "entries": [{ "path": "/w", "sizeBytes": 1, "artifactBytes": 2,
                "sizeMeasured": true, "verdict": { "level": "remove", "freesBytes": 1 } }] }],
        } });
        let b = json!({ "result": {
            "scannedAt": "t2",
            "projects": [{ "entries": [{ "path": "/w", "sizeBytes": 9, "artifactBytes": 0,
                "sizeMeasured": false, "verdict": { "level": "remove", "freesBytes": 9 } }] }],
        } });
        assert_eq!(
            compared("worktree:inventory", a.clone()),
            compared("worktree:inventory", b)
        );
        assert_eq!(
            compared(
                "worktree:removeMany",
                json!({ "result": { "succeeded": ["/w"] } })
            ),
            json!({ "refused": null })
        );
        assert_eq!(
            compared(
                "worktree:pruneOrphans",
                json!({ "error": { "code": 1, "message": "no" } })
            ),
            json!({ "refused": "no" })
        );
        assert_eq!(compared("git:getBranch", a.clone()), a);
    }
}

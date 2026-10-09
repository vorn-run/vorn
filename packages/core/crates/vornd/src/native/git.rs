//! The `git:*` calls vornd answers, for repositories on this machine and in
//! projects on remote hosts, where git runs over ssh as the server's
//! `gitExec` runs it ([`super::remote`]).
//!
//! Each reads its params as the server's handler does and answers what it
//! answers; the git itself is `vorn_git::repo`. Params of a shape the
//! server's handler cannot read are refused ([`super::bad_params`]).

use std::path::Path;

use serde_json::{json, Value};
use vorn_git::repo::{DiffTarget, Done, FullDiff, Git};

use super::remote::Place;
use super::{absolute_str, bad_params, Answer, Native};

/// Answers `method` with `params`.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    let answered = match method {
        // Any path, a relative one included, as the server's `git rev-parse` in it reads it.
        "git:isGitRepo" => params.as_str().map(|p| json!(Place::Local.git(native).is_git_repo(Path::new(p)))),
        "git:listBranches" => project(native, params).map(|(p, place)| {
            let git = place.git(native);
            let dir = Path::new(p);
            // A remote repository is taken to be one, as the server takes it.
            let repo = place.login().is_some() || git.is_git_repo(dir);
            json!({
                "local": if repo { git.list_branches(dir) } else { Vec::new() },
                "current": if repo { git.branch(dir) } else { None },
                "isGitRepo": repo,
            })
        }),
        "git:listRemoteBranches" => {
            project(native, params).map(|(p, place)| json!(place.git(native).list_remote_branches(Path::new(p))))
        }
        "git:listWorktrees" => project(native, params).map(|(p, place)| {
            let list = place.git(native).list_worktrees(Path::new(p));
            Value::Array(
                list.iter()
                    .map(|w| json!({ "path": w.path, "branch": w.branch, "isMain": w.is_main, "name": w.name }))
                    .collect(),
            )
        }),
        "git:getBranch" | "git:getWorktreeBranch" => {
            any_path(native, params).map(|(p, place)| json!(place.git(native).branch(Path::new(p))))
        }
        "git:worktreeDirty" => {
            any_path(native, params).map(|(p, place)| json!(place.git(native).is_worktree_dirty(Path::new(p))))
        }
        "git:diffStat" => any_path(native, params).map(|(p, place)| {
            place
                .git(native)
                .diff_stat(Path::new(p), &DiffTarget::WorkingTree)
                .map_or(Value::Null, |s| stat_json(&s))
        }),
        "git:diffFull" => diff_full(native, params),
        "git:createWorktree" => return create_worktree(native, method, params),
        "git:deleteBranches" => delete_branches(native, params),
        "git:commit" => commit(native, params),
        "git:push" => any_path(native, params).map(|(p, place)| done_json(place.git(native).push(Path::new(p)))),
        _ => return Answer::Unanswered,
    };
    answered.map_or_else(|| bad_params(method), Answer::Result)
}

/// The project path a call names, as a string param, and where it is.
fn project<'a>(native: &Native, params: &'a Value) -> Option<(&'a str, Place)> {
    let path = absolute_str(params)?;
    Some((path, native.project_place(path)))
}

/// Any path a call names, as a string param, and where it is.
fn any_path<'a>(native: &Native, params: &'a Value) -> Option<(&'a str, Place)> {
    let path = absolute_str(params)?;
    Some((path, native.path_place(path)))
}

/// A string field of an object param that is an absolute path.
fn path_field<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    absolute_str(params.get(key)?)
}

/// An optional boolean field: absent is `false`, as the server's defaults
/// read it; anything but a boolean is not a call this answers.
fn flag(params: &Value, key: &str) -> Option<bool> {
    match params.get(key) {
        None | Some(Value::Null) => Some(false),
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => None,
    }
}

fn stat_json(s: &vorn_git::repo::DiffStat) -> Value {
    json!({ "filesChanged": s.files_changed, "insertions": s.insertions, "deletions": s.deletions })
}

fn done_json(done: Done) -> Value {
    match done {
        Done::Ok => json!({ "success": true }),
        Done::Failed(error) => json!({ "success": false, "error": error }),
    }
}

/// `git:diffFull`: a cwd string for the working tree, or `{cwd, from, to}`
/// for a range. A range missing an end reads it as the server formats it.
fn diff_full(native: &Native, params: &Value) -> Option<Value> {
    let (cwd, target) = match params {
        Value::String(_) => (absolute_str(params)?, DiffTarget::WorkingTree),
        Value::Object(map) => {
            let end = |key: &str| match map.get(key) {
                None => Some("undefined".to_owned()),
                Some(Value::String(s)) => Some(s.clone()),
                Some(_) => None,
            };
            (
                path_field(params, "cwd")?,
                DiffTarget::Range {
                    from: end("from")?,
                    to: end("to")?,
                },
            )
        }
        _ => return None,
    };
    Some(
        native
            .path_place(cwd)
            .git(native)
            .diff_full(Path::new(cwd), &target)
            .map_or(Value::Null, |d| full_diff_json(&d)),
    )
}

fn full_diff_json(d: &FullDiff) -> Value {
    json!({
        "stat": stat_json(&d.stat),
        "files": d.files.iter().map(|f| json!({
            "filePath": f.file_path,
            "status": f.status.name(),
            "insertions": f.insertions,
            "deletions": f.deletions,
            "diff": f.diff,
        })).collect::<Vec<_>>(),
    })
}

/// `git:createWorktree {projectPath, branch, worktreeName?}`: git's refusal
/// is the call's error, as the server lets it through.
fn create_worktree(native: &Native, method: &str, params: &Value) -> Answer {
    let Some(project) = path_field(params, "projectPath") else {
        return bad_params(method);
    };
    let Some(branch) = params.get("branch").and_then(Value::as_str) else {
        return bad_params(method);
    };
    let name = match params.get("worktreeName") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(_) => return bad_params(method),
    };
    let place = native.project_place(project);
    let git: Git = place.git(native);
    let made = native.turns.take(&place.turn(project), || {
        git.create_worktree(project, branch, name)
    });
    match made {
        Ok(w) => Answer::Result(json!({
            "worktreePath": w.worktree_path,
            "branch": w.branch,
            "name": w.name,
        })),
        Err(err) => Answer::Error(err.to_string()),
    }
}

/// `git:deleteBranches {projectPath, branches, force?}`.
fn delete_branches(native: &Native, params: &Value) -> Option<Value> {
    let project = path_field(params, "projectPath")?;
    let branches = params
        .get("branches")?
        .as_array()?
        .iter()
        .map(|b| b.as_str().map(str::to_owned))
        .collect::<Option<Vec<String>>>()?;
    let force = flag(params, "force")?;
    let place = native.project_place(project);
    let git = place.git(native);
    let done = native.turns.take(&place.turn(project), || {
        git.delete_branches(Path::new(project), &branches, force)
    });
    let failed: Vec<Value> = done
        .failed
        .iter()
        .map(|(branch, error)| json!({ "branch": branch, "error": error }))
        .collect();
    Some(json!({ "deleted": done.deleted, "failed": failed }))
}

/// `git:commit {cwd, message, includeUnstaged}`.
fn commit(native: &Native, params: &Value) -> Option<Value> {
    let cwd = path_field(params, "cwd")?;
    let message = params.get("message")?.as_str()?;
    let include_unstaged = flag(params, "includeUnstaged")?;
    let place = native.path_place(cwd);
    let git = place.git(native);
    let done = native.turns.take(&place.turn(cwd), || {
        git.commit(Path::new(cwd), message, include_unstaged)
    });
    Some(done_json(done))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_optional_flags_as_the_server_defaults_them() {
        let p = json!({ "force": true, "deleteBranch": null });
        assert_eq!(flag(&p, "force"), Some(true));
        assert_eq!(flag(&p, "deleteBranch"), Some(false));
        assert_eq!(flag(&p, "absent"), Some(false));
        assert_eq!(flag(&json!({ "force": "yes" }), "force"), None);
    }
}

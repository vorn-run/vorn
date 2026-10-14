//! PR checks, promoted from a quick extension. Calls reach Vorn only through
//! `host` and `net`; `gh pr checks` became three requests to GitHub's API,
//! signed by vornd, and the same mapping from a check to its state.

use std::collections::HashMap;

use serde::Serialize;
use vorn_extension::net::fetch;
use vorn_extension::{host, json, Ctx, Error, Extension, Request, Value};

/// One row of the `checks` section, as `views/checks.kdl` declares it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChecksRow {
    pub id: String,
    pub link: String,
    pub name: String,
    pub state: ChecksState,
    pub workflow: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChecksState {
    // In the order a person reads them: failing first.
    Fail,
    Pending,
    Cancel,
    Pass,
    Skipping,
}

pub struct PrChecks;

impl Extension for PrChecks {
    fn handle(method: &str, args: Value, ctx: &Ctx) -> Result<Value, Error> {
        match method {
            "rows" => rows(args, ctx).map(|rows| json!(rows)),
            // Answers without the bridge, for measuring a call.
            "ping" => Ok(Value::Null),
            // What the sandbox must survive: a call that never ends, a panic, an undeclared host.
            "selftest.spin" => loop {
                std::hint::black_box(());
            },
            "selftest.trap" => panic!("a deliberate trap"),
            "selftest.fetch" => {
                fetch(&Request::get("https://example.com/")).map(|r| json!(r.status))
            }
            other => Err(Error::new(
                "unknown-method",
                format!("PR checks has no {other}"),
            )),
        }
    }
}

const API: &str = "https://api.github.com";

fn get(path: &str) -> Result<Value, Error> {
    let req = Request::get(format!("{API}{path}")).header("accept", "application/vnd.github+json");
    fetch(&req)?.json()
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// The rows of `checks`: the current branch's pull request's checks.
fn rows(_args: Value, _ctx: &Ctx) -> Result<Vec<ChecksRow>, Error> {
    let branch = text(&host::call("git.branch", &Value::Null)?);
    let (owner, repo) = owner_repo(&text(&host::call("git.remote", &Value::Null)?))
        .ok_or_else(|| Error::new("not-github", "the origin remote is not on GitHub"))?;
    let base = format!("/repos/{owner}/{repo}");
    let pulls = get(&format!(
        "{base}/pulls?head={owner}:{branch}&state=open&per_page=1"
    ))?;
    let Some(sha) = pulls[0]["head"]["sha"].as_str() else {
        return Ok(Vec::new());
    };
    let runs = get(&format!("{base}/actions/runs?head_sha={sha}&per_page=100"))?;
    let workflow_of: HashMap<u64, String> = runs["workflow_runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r["check_suite_id"].as_u64()?, text(&r["name"]))))
        .collect();
    let checks = get(&format!("{base}/commits/{sha}/check-runs?per_page=100"))?;
    let mut rows: Vec<ChecksRow> = checks["check_runs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let workflow = c["check_suite"]["id"]
                .as_u64()
                .and_then(|id| workflow_of.get(&id).cloned())
                .unwrap_or_default();
            let name = text(&c["name"]);
            ChecksRow {
                id: format!("{workflow}/{name}"),
                link: text(&c["details_url"]),
                state: check_state(c["status"].as_str(), c["conclusion"].as_str()),
                name,
                workflow,
            }
        })
        .collect();
    let statuses = get(&format!("{base}/commits/{sha}/status"))?;
    rows.extend(
        statuses["statuses"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                let name = text(&s["context"]);
                ChecksRow {
                    id: format!("/{name}"),
                    link: text(&s["target_url"]),
                    state: match s["state"].as_str() {
                        Some("success") => ChecksState::Pass,
                        Some("failure" | "error") => ChecksState::Fail,
                        _ => ChecksState::Pending,
                    },
                    name,
                    workflow: String::new(),
                }
            }),
    );
    rows.sort_by(|a, b| (a.state, &a.id).cmp(&(b.state, &b.id)));
    Ok(rows)
}

/// The bucket `gh pr checks` puts a check run in.
fn check_state(status: Option<&str>, conclusion: Option<&str>) -> ChecksState {
    if status != Some("completed") {
        return ChecksState::Pending;
    }
    match conclusion {
        Some("success") => ChecksState::Pass,
        Some("skipped" | "neutral") => ChecksState::Skipping,
        Some("cancelled") => ChecksState::Cancel,
        Some("failure" | "timed_out" | "action_required") => ChecksState::Fail,
        _ => ChecksState::Pending,
    }
}

/// `owner` and `repo` from an https or ssh GitHub remote.
fn owner_repo(remote: &str) -> Option<(String, String)> {
    let path = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("git@github.com:"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))?;
    let (owner, repo) = path
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .split_once('/')?;
    Some((owner.to_owned(), repo.to_owned()))
}

vorn_extension::export!(PrChecks);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_owner_and_repo_from_either_remote() {
        let want = Some(("vorn-run".to_owned(), "vorn".to_owned()));
        assert_eq!(owner_repo("https://github.com/vorn-run/vorn.git"), want);
        assert_eq!(owner_repo("git@github.com:vorn-run/vorn.git"), want);
        assert_eq!(owner_repo("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn buckets_a_check_as_gh_does() {
        assert_eq!(check_state(Some("in_progress"), None), ChecksState::Pending);
        assert_eq!(
            check_state(Some("completed"), Some("neutral")),
            ChecksState::Skipping
        );
        assert_eq!(
            check_state(Some("completed"), Some("timed_out")),
            ChecksState::Fail
        );
    }
}

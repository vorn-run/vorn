//! Worktree cleanup: once a worktree's last session ends it is offered for
//! removal in the sidebar, a worktree's menu removes one (asking first when
//! sessions or changes are at stake), and Settings lists every worktree with
//! what removing it frees.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::{json, Value};
use vorn_extensions::api::types::{Event, RowOp};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::WORKTREE_IDLE;

use super::{arg, unknown, Api, Builtin};
use crate::native::extensions::guard;

const NAME: &str = "Worktree cleanup";
const IDLE: &str = "idle";
const ASKED: &str = "asked";
const ALL: &str = "worktrees";
/// The settings page's scan: whether the first is under way, and why the last failed.
const SCAN: &str = "scan";
/// How often a first scan is tried before the page says it failed: vornd
/// may not have read its sessions yet when the page first asks.
const FIRST_TRIES: u32 = 3;
const RETRY: Duration = Duration::from_secs(1);
/// How long a closed session's processes get to let go of the worktree.
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct Cleanup {
    /// The worktree the confirm dialog asks about.
    asked: Mutex<Option<(String, String)>>,
    /// Whether a scan has listed the worktrees once.
    scanned: AtomicBool,
}

/// A worktree as `worktrees.list` gives it.
struct Found(Value);

impl Found {
    fn path(&self) -> &str {
        self.0["path"].as_str().unwrap_or_default()
    }
    fn project(&self) -> &str {
        self.0["projectPath"].as_str().unwrap_or_default()
    }
    fn flag(&self, key: &str) -> bool {
        self.0[key].as_bool() == Some(true)
    }
    fn sessions(&self) -> Vec<String> {
        self.0["sessionIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    }
    fn name(&self) -> String {
        self.0["name"].as_str().map_or_else(
            || {
                let p = self.path();
                p.rsplit('/').next().unwrap_or(p).to_owned()
            },
            str::to_owned,
        )
    }
    fn branch(&self) -> String {
        self.0["branch"].as_str().unwrap_or("detached").to_owned()
    }
    fn removable(&self) -> bool {
        !self.flag("main") && !self.flag("orphan")
    }

    fn size(&self) -> u64 {
        self.0["sizeBytes"].as_u64().unwrap_or(0)
    }

    /// Its branch and why it is kept or can go: `fix-login · merged · idle 11 days`.
    fn detail(&self) -> String {
        let reasons = self.0["reasons"].as_str().unwrap_or_default();
        let branch = self.branch();
        // The title already reads as the branch when the folder is named for it.
        Some(branch.as_str())
            .filter(|b| *b != self.name())
            .into_iter()
            .chain(reasons.split("; ").filter(|r| !r.is_empty()))
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn settings_row(&self, project_size: u64) -> Value {
        let artifacts = self.0["artifactBytes"].as_u64().unwrap_or(0);
        let dirty = self.flag("dirty");
        json!({
            "id": self.path(),
            "name": self.name(),
            "project": self.0["projectName"],
            "project_path": self.project(),
            "project_size": bytes(project_size),
            "branch": self.branch(),
            "size": bytes(self.size()),
            "detail": self.detail(),
            "verdict": self.0["verdict"],
            "reasons": self.0["reasons"],
            "dirty": dirty,
            "orphan": self.flag("orphan"),
            "reclaimable": artifacts > 0 && !self.flag("orphan"),
            "remove_dirty": self.removable() && dirty,
            "remove_clean": self.removable() && !dirty,
        })
    }
}

/// `1.2 GB`, `340 MB`, `12 KB`.
fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    #[allow(clippy::cast_precision_loss)]
    let mut size = n as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit < 2 || size >= 10.0 {
        format!("{size:.0} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

async fn list(api: &Api, project: Option<&str>, refresh: bool) -> Result<Vec<Found>, ExtError> {
    let all = api
        .call(
            "worktrees.list",
            json!({ "project": project, "refresh": refresh }),
        )
        .await?;
    Ok(all
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| Found(v.clone()))
        .collect())
}

async fn find(api: &Api, path: &str, project: &str) -> Result<Found, ExtError> {
    list(api, Some(project), false)
        .await?
        .into_iter()
        .find(|f| f.path() == path)
        .ok_or_else(|| ExtError::new("gone", format!("{path} is no longer a worktree")))
}

/// The project a row in any of the collections names for worktree `path`.
fn project_of(api: &Api, path: &str) -> Option<String> {
    [IDLE, ALL, ASKED]
        .iter()
        .flat_map(|c| api.rows(c))
        .find(|r| r["id"] == path)
        .and_then(|r| r["project_path"].as_str().map(str::to_owned))
}

/// Closes the worktree's sessions, then removes it, its changes with it.
async fn remove(api: &Api, path: &str, project: &str) -> Result<Value, ExtError> {
    let found = find(api, path, project).await?;
    if !found.removable() {
        return Err(ExtError::new(
            "refused",
            format!("{} is not a worktree Vorn removes", found.name()),
        ));
    }
    let sessions = found.sessions();
    for id in &sessions {
        api.call("sessions.close", json!({ "sessionId": id }))
            .await?;
    }
    if !sessions.is_empty() {
        tokio::time::sleep(SETTLE).await;
    }
    let args = json!({ "worktree": path, "project": project, "force": found.flag("dirty") });
    let answer = api.call("worktrees.remove", args).await?;
    if answer["removed"].as_bool() != Some(true) {
        return Err(ExtError::new("failed", "Failed to remove worktree"));
    }
    for c in [IDLE, ALL] {
        api.publish(c, vec![RowOp::Delete(path.to_owned())]);
    }
    Ok(json!({ "removed": path, "toast": "Worktree removed" }))
}

/// The settings page's rows: every worktree but the projects' own
/// checkouts, by project in the order listed, the largest first in each.
fn settings_rows(found: &[Found]) -> Vec<Value> {
    let mut shown: Vec<&Found> = found.iter().filter(|f| !f.flag("main")).collect();
    let mut order: Vec<&str> = Vec::new();
    for f in &shown {
        if !order.contains(&f.project()) {
            order.push(f.project());
        }
    }
    let at = |p: &str| order.iter().position(|o| *o == p);
    shown.sort_by(|a, b| {
        at(a.project())
            .cmp(&at(b.project()))
            .then(b.size().cmp(&a.size()))
    });
    let total = |p: &str| -> u64 {
        shown
            .iter()
            .filter(|f| f.project() == p)
            .map(|f| f.size())
            .sum()
    };
    shown
        .iter()
        .map(|f| f.settings_row(total(f.project())))
        .collect()
}

fn scan_state(api: &Api, busy: bool, error: Option<String>) {
    let row = json!({ "id": SCAN, "busy": busy, "error": error.unwrap_or_default() });
    api.publish(SCAN, vec![RowOp::Clear, RowOp::Upsert(row)]);
}

async fn scan(api: &Api, refresh: bool) -> Result<Value, ExtError> {
    let found = list(api, None, refresh).await?;
    let mut ops = vec![RowOp::Clear];
    ops.extend(settings_rows(&found).into_iter().map(RowOp::Upsert));
    api.publish(ALL, ops);
    Ok(json!({ "worktrees": found.len() }))
}

impl Cleanup {
    /// Scans for the settings page, trying a first scan again before it
    /// says why it failed; until one lists, the page says it is measuring.
    async fn scan(&self, api: &Api, refresh: bool) -> Result<Value, ExtError> {
        let first = !self.scanned.load(Ordering::Relaxed);
        if first {
            scan_state(api, true, None);
        }
        let tries = if first { FIRST_TRIES } else { 1 };
        let mut tried = 0;
        loop {
            tried += 1;
            match scan(api, refresh).await {
                Ok(v) => {
                    self.scanned.store(true, Ordering::Relaxed);
                    scan_state(api, false, None);
                    return Ok(v);
                }
                Err(_) if tried < tries => tokio::time::sleep(RETRY).await,
                Err(e) => {
                    let why = format!(
                        "Could not measure the worktrees: {}. Refresh to try again.",
                        e.message.trim_end_matches('.')
                    );
                    scan_state(api, false, Some(why));
                    return Err(e);
                }
            }
        }
    }

    async fn idle(&self, api: &Api, data: &Value) {
        let (Some(path), Some(project)) =
            (data["worktreePath"].as_str(), data["projectPath"].as_str())
        else {
            return;
        };
        let Ok(found) = find(api, path, project).await else {
            return;
        };
        if !found.removable() || !found.sessions().is_empty() {
            return;
        }
        let row = json!({
            "id": path,
            "name": found.name(),
            "project": found.0["projectName"],
            "project_path": project,
            "dirty": found.flag("dirty"),
        });
        api.publish(IDLE, vec![RowOp::Upsert(row)]);
    }

    async fn ask(&self, api: &Api, args: &Value) -> Result<Value, ExtError> {
        let path = arg(args, "worktree", "ask")?;
        let project = arg(args, "project", "ask")?;
        let found = find(api, path, project).await?;
        let sessions = found.sessions().len();
        if sessions == 0 && !found.flag("dirty") {
            return remove(api, path, project).await;
        }
        *guard(&self.asked) = Some((path.to_owned(), project.to_owned()));
        let row = json!({
            "id": path,
            "name": found.name(),
            "branch": found.branch(),
            "project_path": project,
            "sessions": sessions,
            "dirty": found.flag("dirty"),
        });
        api.publish(ASKED, vec![RowOp::Clear, RowOp::Upsert(row)]);
        Ok(json!({ "open": "confirm" }))
    }

    async fn act(&self, api: &Api, action: &str, args: Value) -> Result<Value, ExtError> {
        let row = || {
            let path = arg(&args, "id", action)?.to_owned();
            let project = project_of(api, &path)
                .ok_or_else(|| ExtError::new("gone", format!("{path} is not listed")))?;
            Ok::<_, ExtError>((path, project))
        };
        match action {
            "ask" => self.ask(api, &args).await,
            "confirm" => {
                let asked = guard(&self.asked).take();
                api.publish(ASKED, vec![RowOp::Clear]);
                let (path, project) =
                    asked.ok_or_else(|| ExtError::new("gone", "Nothing is asked"))?;
                remove(api, &path, &project).await
            }
            "cancel" => {
                guard(&self.asked).take();
                api.publish(ASKED, vec![RowOp::Clear]);
                Ok(Value::Null)
            }
            "keep" => {
                let path = arg(&args, "id", action)?;
                api.publish(IDLE, vec![RowOp::Delete(path.to_owned())]);
                Ok(Value::Null)
            }
            "remove" => {
                let (path, project) = row()?;
                remove(api, &path, &project).await
            }
            "scan" => self.scan(api, false).await,
            "refresh" => self.scan(api, true).await,
            "reclaim" | "prune" => {
                let (path, _) = row()?;
                api.call(&format!("worktrees.{action}"), json!({ "worktree": path }))
                    .await?;
                scan(api, false).await
            }
            other => Err(unknown(NAME, other)),
        }
    }
}

impl Builtin for Cleanup {
    fn manifest(&self) -> &'static str {
        include_str!("worktrees.toml")
    }

    fn views(&self) -> &[(&'static str, &'static str)] {
        &[
            ("views/idle.kdl", include_str!("views/idle.kdl")),
            ("views/confirm.kdl", include_str!("views/confirm.kdl")),
            ("views/settings.kdl", include_str!("views/settings.kdl")),
        ]
    }

    fn events(&self) -> &'static [&'static str] {
        &[WORKTREE_IDLE]
    }

    fn on_by_default(&self) -> bool {
        true
    }

    fn start<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }

    fn handle<'a>(
        &'a self,
        api: &'a Api,
        action: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        self.act(api, action, args).boxed()
    }

    fn on_events<'a>(&'a self, api: &'a Api, batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async move {
            for event in batch.iter().filter(|e| e.name == WORKTREE_IDLE) {
                self.idle(api, &event.data).await;
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::bytes;

    #[test]
    fn sizes_read_in_the_largest_whole_unit() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(12 * 1024), "12 KB");
        assert_eq!(bytes(340 * 1024 * 1024), "340 MB");
        assert_eq!(bytes(9 * 1024 * 1024), "9.0 MB");
        assert_eq!(bytes(1288 * 1024 * 1024), "1.3 GB");
    }
}

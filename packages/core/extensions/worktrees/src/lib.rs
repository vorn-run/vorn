//! Worktree cleanup: once a worktree's last session ends it is offered for
//! removal in the sidebar, a worktree's menu removes one (asking first when
//! sessions or changes are at stake), and Settings lists every worktree with
//! what removing it frees, drawn as an element tree.
//!
//! Vorn ships it as a Wasm component (`vorn-extension.toml` beside this
//! crate): everything it knows comes through the extension API, and what it
//! shows lives in the rows it publishes, so a restart of its instance loses
//! nothing a person sees.

use std::time::Duration;

use vorn_extension_sdk::tree::{self, Surface};
use vorn_extension_sdk::{arg, json, Ctx, Error, Event, Extension, Host, Init, Op, Value};

mod page;

const NAME: &str = "Worktree cleanup";
/// The event a worktree's last session ending sends.
const WORKTREE_IDLE: &str = "worktree.idle";
const IDLE: &str = "idle";
const ASKED: &str = "asked";
const ALL: &str = "worktrees";
/// The settings page's scan: whether the first is under way, and why the last failed.
const SCAN: &str = "scan";
/// The settings page's totals: what is on disk, what is build output, what can go.
const SUMMARY: &str = "summary";
/// How often a first scan is tried before the page says it failed: vornd
/// may not have read its sessions yet when the page first asks.
const FIRST_TRIES: u32 = 3;
const RETRY: Duration = Duration::from_secs(1);
/// How long a closed session's processes get to let go of the worktree.
const SETTLE: Duration = Duration::from_millis(500);

/// The extension. The worktree its dialog asks about is the `asked` row.
#[derive(Debug, Default)]
pub struct Cleanup {
    /// Whether a scan has listed the worktrees once.
    scanned: bool,
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

    fn artifacts(&self) -> u64 {
        self.0["artifactBytes"].as_u64().unwrap_or(0)
    }

    /// No session holds it, so the page may act on it in bulk.
    fn free(&self) -> bool {
        !self.flag("main") && self.sessions().is_empty()
    }

    /// Its build output can go: it has some and is a worktree git knows.
    fn reclaimable(&self) -> bool {
        self.artifacts() > 0 && !self.flag("orphan")
    }

    /// Removing it is what the inventory advises.
    fn advised(&self) -> bool {
        matches!(self.0["verdict"].as_str(), Some("remove" | "orphan"))
    }

    /// How long it has sat idle, then why it is kept or can go: `today · uncommitted changes`.
    fn detail(&self) -> String {
        let age = match self.0["idleDays"].as_u64() {
            Some(0) => Some("today".to_owned()),
            Some(1) => Some("idle 1 day".to_owned()),
            Some(n) => Some(format!("idle {n} days")),
            None => None,
        };
        let reasons = self.0["reasons"].as_str().unwrap_or_default();
        age.into_iter()
            .chain(
                reasons
                    .split("; ")
                    .filter(|r| !r.is_empty() && !r.starts_with("idle "))
                    .map(str::to_owned),
            )
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn verdict_label(&self) -> &'static str {
        match self.0["verdict"].as_str() {
            Some("remove") => "Removable",
            Some("reclaim") => "Build output only",
            Some("review") => "Review",
            Some("orphan") => "Orphan folder",
            _ => "Keep",
        }
    }

    fn settings_row(&self, project_size: u64) -> Value {
        let dirty = self.flag("dirty");
        let against = self.0["base"]
            .as_str()
            .filter(|b| !b.is_empty())
            .map(|b| format!("vs {b}"))
            .unwrap_or_default();
        json!({
            "id": self.path(),
            "name": self.name(),
            "project": self.0["projectName"],
            "project_path": self.project(),
            "project_size": bytes(project_size),
            "against": against,
            "branch": self.branch(),
            "size": bytes(self.size()),
            "size_bytes": self.size(),
            "artifact_bytes": self.artifacts(),
            "detail": self.detail(),
            "verdict": self.0["verdict"],
            "verdict_label": self.verdict_label(),
            "reasons": self.0["reasons"],
            "dirty": dirty,
            "orphan": self.flag("orphan"),
            "reclaimable": self.reclaimable(),
            "held": !self.free(),
            "remove_dirty": self.removable() && dirty,
            "remove_clean": self.removable() && !dirty,
        })
    }
}

/// The page's totals: all on disk, and of what no session holds, build output and removable.
fn summary_row(shown: &[&Found]) -> Value {
    let total: u64 = shown.iter().map(|f| f.size()).sum();
    let free: Vec<&&Found> = shown.iter().filter(|f| f.free()).collect();
    let targets: Vec<&&Found> = free.iter().copied().filter(|f| f.reclaimable()).collect();
    let build: u64 = targets.iter().map(|f| f.artifacts()).sum();
    let removable: u64 = free.iter().filter(|f| f.advised()).map(|f| f.size()).sum();
    let n = targets.len();
    json!({
        "id": SUMMARY,
        "total": bytes(total),
        "total_bytes": total,
        "build": bytes(build),
        "build_bytes": build,
        "removable": bytes(removable),
        "removable_bytes": removable,
        "reclaimable": n > 0,
        "reclaim_label": format!("Reclaim {} of build output", bytes(build)),
        "reclaim_confirm": format!(
            "Delete the build output in {n} worktree{}? This frees {}.",
            if n == 1 { "" } else { "s" },
            bytes(build)
        ),
    })
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

fn list(host: &mut dyn Host, project: Option<&str>, refresh: bool) -> Result<Vec<Found>, Error> {
    let all = host.call(
        "worktrees.list",
        json!({ "project": project, "refresh": refresh }),
    )?;
    Ok(match all {
        Value::Array(found) => found.into_iter().map(Found).collect(),
        _ => Vec::new(),
    })
}

fn find(host: &mut dyn Host, path: &str, project: &str) -> Result<Found, Error> {
    list(host, Some(project), false)?
        .into_iter()
        .find(|f| f.path() == path)
        .ok_or_else(|| Error::new("gone", format!("{path} is no longer a worktree")))
}

/// The project a row in any of the collections names for worktree `path`.
fn project_of(host: &mut dyn Host, path: &str) -> Option<String> {
    [IDLE, ALL, ASKED]
        .iter()
        .flat_map(|c| host.rows(c).unwrap_or_default())
        .find(|r| r["id"] == path)
        .and_then(|r| r["project_path"].as_str().map(str::to_owned))
}

/// Closes the worktree's sessions, then removes it, its changes with it.
fn remove(host: &mut dyn Host, path: &str, project: &str) -> Result<Value, Error> {
    let found = find(host, path, project)?;
    if !found.removable() {
        return Err(Error::new(
            "refused",
            format!("{} is not a worktree Vorn removes", found.name()),
        ));
    }
    let sessions = found.sessions();
    for id in &sessions {
        host.call("sessions.close", json!({ "sessionId": id }))?;
    }
    if !sessions.is_empty() {
        std::thread::sleep(SETTLE);
    }
    let args = json!({ "worktree": path, "project": project, "force": found.flag("dirty") });
    let answer = host.call("worktrees.remove", args)?;
    if answer["removed"].as_bool() != Some(true) {
        return Err(Error::new("failed", "Failed to remove worktree"));
    }
    for c in [IDLE, ALL] {
        host.publish(c, vec![Op::Delete(path.to_owned())]);
    }
    Ok(json!({ "removed": path, "toast": "Worktree removed" }))
}

/// The settings page's rows, by project with the largest first, and its totals.
fn settings_rows(found: &[Found]) -> (Vec<Value>, Value) {
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
    let rows = shown
        .iter()
        .map(|f| f.settings_row(total(f.project())))
        .collect();
    (rows, summary_row(&shown))
}

fn scan_state(host: &mut dyn Host, busy: bool, error: Option<String>) {
    let row = json!({ "id": SCAN, "busy": busy, "error": error.unwrap_or_default() });
    host.publish(SCAN, vec![Op::Clear, Op::Upsert(row)]);
}

fn scan(host: &mut dyn Host, refresh: bool) -> Result<Value, Error> {
    let found = list(host, None, refresh)?;
    let (rows, summary) = settings_rows(&found);
    let mut ops = vec![Op::Clear];
    ops.extend(rows.into_iter().map(Op::Upsert));
    host.publish(ALL, ops);
    host.publish(SUMMARY, vec![Op::Clear, Op::Upsert(summary)]);
    Ok(json!({ "worktrees": found.len() }))
}

/// Deletes the build output of every worktree no session holds.
fn reclaim_all(host: &mut dyn Host) -> Result<Value, Error> {
    let found = list(host, None, false)?;
    let mut done = 0;
    for f in found.iter().filter(|f| f.free() && f.reclaimable()) {
        host.call("worktrees.reclaim", json!({ "worktree": f.path() }))?;
        done += 1;
    }
    scan(host, false)?;
    let toast = format!(
        "Deleted the build output in {done} worktree{}",
        if done == 1 { "" } else { "s" }
    );
    Ok(json!({ "reclaimed": done, "toast": toast }))
}

/// Removes the checked worktrees; one that fails stops the rest, then the list is rescanned.
fn remove_many(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
    let ids: Vec<String> = args["ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    if ids.is_empty() {
        return Err(Error::new("bad_args", "remove_many needs ids"));
    }
    let rows = host.rows(ALL)?;
    let mut done = 0;
    let mut failed = None;
    for id in &ids {
        let Some(row) = rows.iter().find(|r| r["id"] == id.as_str()) else {
            continue;
        };
        let result = if row["orphan"].as_bool() == Some(true) {
            host.call("worktrees.prune", json!({ "worktree": id }))
                .map(|_| ())
        } else {
            let project = row["project_path"].as_str().unwrap_or_default();
            remove(host, id, project).map(|_| ())
        };
        match result {
            Ok(()) => done += 1,
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    scan(host, false)?;
    if let Some(e) = failed {
        return Err(e);
    }
    let toast = format!(
        "Removed {done} worktree{}",
        if done == 1 { "" } else { "s" }
    );
    Ok(json!({ "removed": done, "toast": toast }))
}

impl Cleanup {
    /// Scans for the settings page, trying a first scan again before it
    /// says why it failed; until one lists, the page says it is measuring.
    fn scan(&mut self, host: &mut dyn Host, refresh: bool) -> Result<Value, Error> {
        let first = !self.scanned;
        if first {
            scan_state(host, true, None);
        }
        let tries = if first { FIRST_TRIES } else { 1 };
        let mut tried = 0;
        loop {
            tried += 1;
            match scan(host, refresh) {
                Ok(v) => {
                    self.scanned = true;
                    scan_state(host, false, None);
                    return Ok(v);
                }
                Err(_) if tried < tries => std::thread::sleep(RETRY),
                Err(e) => {
                    let why = format!(
                        "Could not measure the worktrees: {}. Refresh to try again.",
                        e.message.trim_end_matches('.')
                    );
                    scan_state(host, false, Some(why));
                    return Err(e);
                }
            }
        }
    }

    fn idle(host: &mut dyn Host, data: &Value) {
        let (Some(path), Some(project)) =
            (data["worktreePath"].as_str(), data["projectPath"].as_str())
        else {
            return;
        };
        let Ok(found) = find(host, path, project) else {
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
        host.publish(IDLE, vec![Op::Upsert(row)]);
    }

    fn ask(host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
        let path = arg(args, "worktree", "ask")?;
        let project = arg(args, "project", "ask")?;
        let found = find(host, path, project)?;
        let sessions = found.sessions().len();
        if sessions == 0 && !found.flag("dirty") {
            return remove(host, path, project);
        }
        let row = json!({
            "id": path,
            "name": found.name(),
            "branch": found.branch(),
            "project_path": project,
            "sessions": sessions,
            "dirty": found.flag("dirty"),
        });
        host.publish(ASKED, vec![Op::Clear, Op::Upsert(row)]);
        Ok(json!({ "open": "confirm" }))
    }

    /// The worktree the dialog asks about, which the `asked` row holds.
    fn asked(host: &mut dyn Host) -> Option<(String, String)> {
        let row = host.rows(ASKED).ok()?.into_iter().next()?;
        let path = row["id"].as_str()?.to_owned();
        let project = row["project_path"].as_str()?.to_owned();
        Some((path, project))
    }

    fn act(&mut self, host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        let row = |host: &mut dyn Host| {
            let path = arg(args, "id", action)?.to_owned();
            let project = project_of(host, &path)
                .ok_or_else(|| Error::new("gone", format!("{path} is not listed")))?;
            Ok::<_, Error>((path, project))
        };
        match action {
            "ask" => Cleanup::ask(host, args),
            "confirm" => {
                let asked = Cleanup::asked(host);
                host.publish(ASKED, vec![Op::Clear]);
                let (path, project) =
                    asked.ok_or_else(|| Error::new("gone", "Nothing is asked"))?;
                remove(host, &path, &project)
            }
            "cancel" => {
                host.publish(ASKED, vec![Op::Clear]);
                Ok(Value::Null)
            }
            "keep" => {
                let path = arg(args, "id", action)?;
                host.publish(IDLE, vec![Op::Delete(path.to_owned())]);
                Ok(Value::Null)
            }
            "remove" => {
                let (path, project) = row(host)?;
                remove(host, &path, &project)
            }
            "scan" => self.scan(host, false),
            "reclaim_all" => reclaim_all(host),
            "remove_many" => remove_many(host, args),
            "refresh" => self.scan(host, true),
            "reclaim" | "prune" => {
                let (path, _) = row(host)?;
                host.call(&format!("worktrees.{action}"), json!({ "worktree": path }))?;
                scan(host, false)
            }
            other => Err(Error::new(
                "unknown-action",
                format!("{NAME} has no action {other}"),
            )),
        }
    }
}

impl Extension for Cleanup {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Cleanup, Error> {
        Ok(Cleanup::default())
    }

    fn handle(
        &mut self,
        host: &mut dyn Host,
        method: &str,
        args: Value,
        _: &Ctx,
    ) -> Result<Value, Error> {
        self.act(host, method, &args)
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: Vec<Event>) {
        for event in batch.iter().filter(|e| e.name == WORKTREE_IDLE) {
            Cleanup::idle(host, &event.data);
        }
    }

    fn render(&mut self, host: &mut dyn Host, _: &str, _: Surface) -> Result<tree::Node, Error> {
        page::render(host)
    }

    fn event(&mut self, host: &mut dyn Host, _: &str, event: tree::Event) -> Result<Value, Error> {
        let (does, args) = page::event(&event)?;
        self.act(host, does, &args)
    }
}

vorn_extension_sdk::export!(Cleanup);

#[cfg(test)]
mod tests;

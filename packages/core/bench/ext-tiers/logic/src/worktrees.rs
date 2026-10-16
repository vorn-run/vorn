//! Worktree cleanup, ported from vornd's built-in: once a worktree's last
//! session ends it is offered for removal in the sidebar, a worktree's menu
//! removes one (asking first when sessions or changes are at stake), and
//! Settings lists every worktree with what removing it frees.

use std::time::Duration;

use serde_json::{json, Value};

use crate::{arg, unknown, Error, Extension, Host, Op, Rows};

const NAME: &str = "Worktree cleanup";
const IDLE: &str = "idle";
const ASKED: &str = "asked";
const ALL: &str = "worktrees";
/// The public event a worktree's last session ending sends.
pub const WORKTREE_IDLE: &str = "worktree.idle";
/// How long a closed session's processes get to let go of the worktree.
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct Cleanup {
    /// The worktree the confirm dialog asks about.
    asked: Option<(String, String)>,
    rows: Rows,
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

    fn settings_row(&self) -> Value {
        let artifacts = self.0["artifactBytes"].as_u64().unwrap_or(0);
        let dirty = self.flag("dirty");
        json!({
            "id": self.path(),
            "name": self.name(),
            "project": self.0["projectName"],
            "project_path": self.project(),
            "branch": self.branch(),
            "size": bytes(self.0["sizeBytes"].as_u64().unwrap_or(0)),
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
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    #[allow(clippy::cast_precision_loss)]
    let mut size = n as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit < 2 {
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
    Ok(all
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| Found(v.clone()))
        .collect())
}

fn find(host: &mut dyn Host, path: &str, project: &str) -> Result<Found, Error> {
    list(host, Some(project), false)?
        .into_iter()
        .find(|f| f.path() == path)
        .ok_or_else(|| Error::new("gone", format!("{path} is no longer a worktree")))
}

impl Cleanup {
    /// The project a row in any of the collections names for worktree `path`.
    fn project_of(&self, path: &str) -> Option<String> {
        [IDLE, ALL, ASKED]
            .iter()
            .flat_map(|c| self.rows.rows(c))
            .find(|r| r["id"] == path)
            .and_then(|r| r["project_path"].as_str().map(str::to_owned))
    }

    /// Closes the worktree's sessions, then removes it, its changes with it.
    fn remove(&mut self, host: &mut dyn Host, path: &str, project: &str) -> Result<Value, Error> {
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
            self.rows
                .publish(host, c, vec![Op::Delete(path.to_owned())]);
        }
        Ok(json!({ "removed": path, "toast": "Worktree removed" }))
    }

    fn scan(&mut self, host: &mut dyn Host, refresh: bool) -> Result<Value, Error> {
        let found = list(host, None, refresh)?;
        let mut ops = vec![Op::Clear];
        ops.extend(found.iter().map(|f| Op::Upsert(f.settings_row())));
        self.rows.publish(host, ALL, ops);
        Ok(json!({ "worktrees": found.len() }))
    }

    fn idle(&mut self, host: &mut dyn Host, data: &Value) {
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
        self.rows.publish(host, IDLE, vec![Op::Upsert(row)]);
    }

    fn ask(&mut self, host: &mut dyn Host, args: &Value) -> Result<Value, Error> {
        let path = arg(args, "worktree", "ask")?;
        let project = arg(args, "project", "ask")?;
        let found = find(host, path, project)?;
        let sessions = found.sessions().len();
        if sessions == 0 && !found.flag("dirty") {
            return self.remove(host, path, project);
        }
        self.asked = Some((path.to_owned(), project.to_owned()));
        let row = json!({
            "id": path,
            "name": found.name(),
            "branch": found.branch(),
            "project_path": project,
            "sessions": sessions,
            "dirty": found.flag("dirty"),
        });
        self.rows
            .publish(host, ASKED, vec![Op::Clear, Op::Upsert(row)]);
        Ok(json!({ "open": "confirm" }))
    }

    fn row(&self, args: &Value, action: &str) -> Result<(String, String), Error> {
        let path = arg(args, "id", action)?.to_owned();
        let project = self
            .project_of(&path)
            .ok_or_else(|| Error::new("gone", format!("{path} is not listed")))?;
        Ok((path, project))
    }
}

impl Extension for Cleanup {
    fn init(&mut self, _host: &mut dyn Host) {}

    fn handle(&mut self, host: &mut dyn Host, action: &str, args: &Value) -> Result<Value, Error> {
        match action {
            "ask" => self.ask(host, args),
            "confirm" => {
                let asked = self.asked.take();
                self.rows.publish(host, ASKED, vec![Op::Clear]);
                let (path, project) =
                    asked.ok_or_else(|| Error::new("gone", "Nothing is asked"))?;
                self.remove(host, &path, &project)
            }
            "cancel" => {
                self.asked = None;
                self.rows.publish(host, ASKED, vec![Op::Clear]);
                Ok(Value::Null)
            }
            "keep" => {
                let path = arg(args, "id", action)?;
                self.rows
                    .publish(host, IDLE, vec![Op::Delete(path.to_owned())]);
                Ok(Value::Null)
            }
            "remove" => {
                let (path, project) = self.row(args, action)?;
                self.remove(host, &path, &project)
            }
            "scan" => self.scan(host, false),
            "refresh" => self.scan(host, true),
            "reclaim" | "prune" => {
                let (path, _) = self.row(args, action)?;
                host.call(&format!("worktrees.{action}"), json!({ "worktree": path }))?;
                self.scan(host, false)
            }
            other => Err(unknown(NAME, other)),
        }
    }

    fn on_events(&mut self, host: &mut dyn Host, batch: &[(String, Value)]) {
        for (_, data) in batch.iter().filter(|(name, _)| name == WORKTREE_IDLE) {
            self.idle(host, data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::bytes;

    #[test]
    fn sizes_read_in_the_largest_whole_unit() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(12 * 1024), "12 KB");
        assert_eq!(bytes(340 * 1024 * 1024), "340.0 MB");
        assert_eq!(bytes(1288 * 1024 * 1024), "1.3 GB");
    }
}

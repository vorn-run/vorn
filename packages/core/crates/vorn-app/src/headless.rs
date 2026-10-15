//! The headless agents the toolbar badge lists: `headless:list` polled and
//! kept with what `headless:exit` and `headless:data` say between polls,
//! as the renderer's terminals slice keeps them.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::state::Filter;

/// How often `headless:list` is asked again.
pub const POLL: std::time::Duration = std::time::Duration::from_secs(5);
/// How often exited sessions past their retention are dropped.
pub const PRUNE: std::time::Duration = std::time::Duration::from_secs(30);
/// An expanded pill's log is cut back to `LOG_KEEP` once over `LOG_CAP`.
const LOG_CAP: usize = 100_000;
const LOG_KEEP: usize = 80_000;

/// `useFilteredHeadless` over the app's settings, project and workspace.
pub fn shown<'a>(store: &crate::client::Store, st: &'a crate::state::State) -> Vec<&'a Headless> {
    let show = store
        .config
        .pointer("/defaults/showHeadlessAgents")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let workspace: HashSet<&str> = store
        .workspace_projects()
        .map(|p| p.name.as_str())
        .collect();
    st.headless
        .visible(show, st.project.as_deref(), Some(&workspace), st.filter)
}

/// An expanded pill's log, which scrolls.
pub fn log_id(id: &str) -> crate::ui::Id {
    crate::ui::Id::new("headless-log").child(id)
}

/// One of `headless:list`'s sessions.
#[derive(Debug, Clone, PartialEq)]
pub struct Headless {
    pub id: String,
    /// `displayName`, else the project's name.
    pub name: String,
    pub project: String,
    pub agent: String,
    pub branch: Option<String>,
    /// `running` or `exited`.
    pub status: String,
    pub exit: Option<i64>,
    pub started: f64,
    pub ended: Option<f64>,
    /// What started it, when not a person: a workflow, an extension.
    pub origin: Option<Origin>,
}

/// A session record's `origin`: the extension, its id for the thing, the words shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    pub extension: String,
    pub id: String,
    pub label: String,
}

/// The extension a workflow's sessions name as their origin.
pub const WORKFLOWS: &str = "vorn.workflows";

impl Headless {
    pub fn from_value(v: &Value) -> Option<Headless> {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let project = s("projectName").unwrap_or_default();
        Some(Headless {
            id: s("id")?,
            name: s("displayName").unwrap_or_else(|| project.clone()),
            project,
            agent: s("agentType").unwrap_or_else(|| "shell".to_owned()),
            branch: s("branch"),
            status: s("status").unwrap_or_else(|| "running".to_owned()),
            exit: v.get("exitCode").and_then(Value::as_i64),
            started: v.get("startedAt").and_then(Value::as_f64).unwrap_or(0.0),
            ended: v.get("endedAt").and_then(Value::as_f64),
            origin: v.get("origin").and_then(|o| {
                let t = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_owned);
                Some(Origin {
                    extension: t("extension")?,
                    id: t("id")?,
                    label: t("label")?,
                })
            }),
        })
    }

    pub fn running(&self) -> bool {
        self.status == "running"
    }

    /// Exited with anything but 0, or with no code at all.
    pub fn errored(&self) -> bool {
        self.status == "exited" && self.exit != Some(0)
    }

    /// How long it ran, or has been running at `now`.
    pub fn duration(&self, now: f64) -> String {
        let end = if self.running() {
            now
        } else {
            self.ended.unwrap_or(now)
        };
        format_duration(end - self.started)
    }

    /// The status filter's bucket: running, error or idle.
    fn bucket(&self) -> Filter {
        if self.running() {
            Filter::Running
        } else if self.exit != Some(0) {
            Filter::Error
        } else {
            Filter::Idle
        }
    }
}

/// `formatDuration`: 45s, 3m12s, 3m, 1h5m.
pub fn format_duration(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    if s < 60 {
        return format!("{s}s");
    }
    let (m, rs) = (s / 60, s % 60);
    if m < 60 {
        return if rs > 0 {
            format!("{m}m{rs}s")
        } else {
            format!("{m}m")
        };
    }
    format!("{}h{}m", m / 60, m % 60)
}

/// The sessions with their last output line, the expanded pills' logs and
/// those hidden.
#[derive(Debug, Default)]
pub struct Tracker {
    list: Vec<Headless>,
    last: HashMap<String, String>,
    logs: HashMap<String, String>,
    dismissed: HashSet<String>,
    expanded: HashSet<String>,
    /// Logs that grew since the frame last scrolled them to the end.
    grew: HashSet<String>,
}

impl Tracker {
    pub fn iter(&self) -> std::slice::Iter<'_, Headless> {
        self.list.iter()
    }

    /// `setHeadlessSessions`: the server's list, keeping what it has
    /// dropped while running or since it exited.
    pub fn sync(&mut self, sessions: Vec<Headless>) {
        let ids: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        let kept: Vec<Headless> = self
            .list
            .iter()
            .filter(|s| !ids.contains(s.id.as_str()) && !self.dismissed.contains(&s.id))
            .filter(|s| s.running() || (s.status == "exited" && s.ended.is_some()))
            .cloned()
            .collect();
        let mut next = Vec::with_capacity(sessions.len() + kept.len());
        for s in sessions {
            if self.dismissed.contains(&s.id) {
                continue;
            }
            match self.list.iter().find(|p| p.id == s.id) {
                Some(prev) => next.push(Headless {
                    status: s.status,
                    exit: s.exit,
                    ended: s.ended,
                    ..prev.clone()
                }),
                None => next.push(s),
            }
        }
        next.extend(kept);
        self.list = next;
        self.forget_gone();
    }

    /// `headless:exit`.
    pub fn exited(&mut self, id: &str, code: Option<i64>, now: f64) {
        if let Some(s) = self.list.iter_mut().find(|s| s.id == id) {
            s.status = "exited".to_owned();
            s.exit = code;
            s.ended = Some(now);
        }
    }

    /// `headless:data`: its last non-blank line, and the log while expanded.
    pub fn data(&mut self, id: &str, data: &str) {
        if let Some(line) = data.split('\n').rfind(|l| !l.trim().is_empty()) {
            self.last.insert(id.to_owned(), line.to_owned());
        }
        if let Some(log) = self.logs.get_mut(id) {
            log.push_str(data);
            self.grew.insert(id.to_owned());
            if log.len() > LOG_CAP {
                let mut cut = log.len() - LOG_KEEP;
                while !log.is_char_boundary(cut) {
                    cut += 1;
                }
                log.drain(..cut);
            }
        }
    }

    /// `dismissHeadlessSession`: hidden, and not brought back by a poll.
    pub fn dismiss(&mut self, id: &str) {
        self.dismissed.insert(id.to_owned());
        self.list.retain(|s| s.id != id);
        self.forget_gone();
    }

    /// `pruneExitedHeadless`: exited sessions older than `retention_ms` go.
    pub fn prune(&mut self, retention_ms: f64, now: f64) {
        let gone: Vec<String> = self
            .list
            .iter()
            .filter(|s| !s.running() && s.ended.is_some_and(|e| now - e >= retention_ms))
            .map(|s| s.id.clone())
            .collect();
        if gone.is_empty() {
            return;
        }
        self.list.retain(|s| !gone.contains(&s.id));
        for id in &gone {
            self.dismissed.remove(id);
        }
        self.forget_gone();
    }

    pub fn last(&self, id: &str) -> Option<&str> {
        self.last.get(id).map(String::as_str)
    }

    pub fn expanded(&self, id: &str) -> bool {
        self.expanded.contains(id)
    }

    /// What an expanded pill collected since it opened.
    pub fn log(&self, id: &str) -> &str {
        self.logs.get(id).map_or("", String::as_str)
    }

    /// Expands or collapses a pill; its log starts when it opens.
    pub fn toggle(&mut self, id: &str) {
        if self.expanded.remove(id) {
            self.logs.remove(id);
        } else {
            self.expanded.insert(id.to_owned());
            self.logs.insert(id.to_owned(), String::new());
        }
    }

    /// The logs that grew, to keep at their end.
    pub fn take_grew(&mut self) -> HashSet<String> {
        std::mem::take(&mut self.grew)
    }

    /// The popover closed: every pill folds back.
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
        self.logs.clear();
    }

    /// `useFilteredHeadless`: hidden when the setting is off, else the
    /// active project's (or the workspace's) in the status filter.
    pub fn visible<'a>(
        &'a self,
        show: bool,
        project: Option<&str>,
        workspace: Option<&HashSet<&str>>,
        filter: Filter,
    ) -> Vec<&'a Headless> {
        if !show {
            return Vec::new();
        }
        self.list
            .iter()
            .filter(|s| match project {
                Some(p) => s.project == p,
                None => workspace.is_none_or(|w| w.contains(s.project.as_str())),
            })
            .filter(|s| filter == Filter::All || s.bucket() == filter)
            .collect()
    }

    fn forget_gone(&mut self) {
        let ids: HashSet<&str> = self.list.iter().map(|s| s.id.as_str()).collect();
        self.last.retain(|k, _| ids.contains(k.as_str()));
        self.logs.retain(|k, _| ids.contains(k.as_str()));
        self.expanded.retain(|k| ids.contains(k.as_str()));
        self.grew.retain(|k| ids.contains(k.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn h(id: &str, status: &str, exit: Option<i64>, ended: Option<f64>) -> Headless {
        Headless::from_value(&json!({
            "id": id, "agentType": "claude", "projectName": "web", "status": status,
            "exitCode": exit, "startedAt": 1_000.0, "endedAt": ended,
        }))
        .expect("session")
    }

    #[test]
    fn a_record_reads_as_vornd_writes_it() {
        let s = Headless::from_value(&json!({
            "id": "h1", "pid": 4, "agentType": "codex", "projectName": "web", "projectPath": "/w",
            "branch": "main", "status": "exited", "exitCode": 2, "startedAt": 1_000,
            "endedAt": 4_000,
            "origin": { "extension": "vorn.workflows", "id": "wf", "label": "Ship" }
        }))
        .expect("session");
        assert_eq!(s.name, "web", "the project names it without a displayName");
        assert_eq!(
            (s.started, s.ended, s.exit),
            (1_000.0, Some(4_000.0), Some(2))
        );
        assert_eq!(s.origin.as_ref().map(|o| o.label.as_str()), Some("Ship"));
        assert!(s.errored() && !s.running());
        assert_eq!(s.duration(99_000.0), "3s", "an exited one stops at its end");
    }

    #[test]
    fn durations_read_as_the_renderer_writes_them() {
        assert_eq!(format_duration(45_000.0), "45s");
        assert_eq!(format_duration(192_000.0), "3m12s");
        assert_eq!(format_duration(180_000.0), "3m");
        assert_eq!(format_duration(3_900_000.0), "1h5m");
        assert_eq!(format_duration(-5.0), "0s");
    }

    #[test]
    fn a_poll_keeps_what_the_server_dropped_and_hides_the_dismissed() {
        let mut t = Tracker::default();
        t.sync(vec![
            h("a", "running", None, None),
            h("b", "running", None, None),
        ]);
        t.data("a", "one\ntwo\n\n");
        assert_eq!(t.last("a"), Some("two"));
        t.exited("b", Some(0), 9_000.0);
        t.dismiss("a");
        t.sync(vec![h("a", "running", None, None)]);
        let ids: Vec<&str> = t.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["b"], "a stays hidden; b exited, so it is kept");
        assert_eq!(t.last("a"), None);
        t.prune(60_000.0, 30_000.0);
        assert_eq!(t.iter().count(), 1, "within its retention");
        t.prune(60_000.0, 69_000.0);
        assert_eq!(t.iter().count(), 0);
    }

    #[test]
    fn an_expanded_pill_logs_from_when_it_opens_and_caps_it() {
        let mut t = Tracker::default();
        t.sync(vec![h("a", "running", None, None)]);
        t.data("a", "before\n");
        t.toggle("a");
        assert!(t.expanded("a"));
        assert_eq!(t.log("a"), "");
        t.data("a", &"x".repeat(LOG_CAP));
        t.data("a", "tail");
        assert_eq!(t.log("a").len(), LOG_KEEP);
        assert!(t.log("a").ends_with("tail"));
        t.toggle("a");
        assert_eq!(t.log("a"), "");
    }

    #[test]
    fn the_filters_follow_the_project_workspace_and_status() {
        let mut t = Tracker::default();
        let mut other = h("c", "running", None, None);
        other.project = "api".into();
        t.sync(vec![
            h("a", "running", None, None),
            h("b", "exited", Some(1), Some(2.0)),
            other,
            h("d", "exited", Some(0), Some(2.0)),
        ]);
        let ids = |v: Vec<&Headless>| v.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        assert!(t.visible(false, None, None, Filter::All).is_empty());
        assert_eq!(ids(t.visible(true, Some("api"), None, Filter::All)), ["c"]);
        let ws: HashSet<&str> = ["web"].into();
        assert_eq!(
            ids(t.visible(true, None, Some(&ws), Filter::All)),
            ["a", "b", "d"]
        );
        assert_eq!(ids(t.visible(true, None, None, Filter::Error)), ["b"]);
        assert_eq!(ids(t.visible(true, None, None, Filter::Idle)), ["d"]);
        assert!(t.visible(true, None, None, Filter::Waiting).is_empty());
    }
}

//! Sessions: the terminal sessions saved across restarts, the schedule log,
//! effect receipts and the per-session event log.

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Params, Row, Statement};
use vorn_protocol::{
    AgentStatus, AgentType, ScheduleLogEntry, SessionEvent, SessionEventType, TerminalSession,
};

use crate::sql::{
    get_f64, get_opt_f64, get_opt_text, get_text, json_if_truthy, now_millis, num, parse_json,
};
use crate::{Result, Store};

/// Schedule log entries kept, oldest dropped first.
const MAX_LOG_ENTRIES: i64 = 200;

/// Session events kept per session, newest by timestamp.
const MAX_SESSION_EVENTS_PER_SESSION: i64 = 200;

/// How many session events a listing returns when the caller gives no limit.
const DEFAULT_SESSION_EVENT_LIMIT: f64 = 100.0;

/// Every row a statement returns, each mapped by `map`.
fn collect<T>(
    stmt: &mut Statement<'_>,
    params: impl Params,
    map: impl Fn(&Row<'_>) -> Result<T>,
) -> Result<Vec<T>> {
    let mut rows = stmt.query(params)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map(row)?);
    }
    Ok(out)
}

/// A nonzero integer flag column: `Some(true)` when set, else left out.
fn flag(row: &Row<'_>, name: &str) -> Result<Option<bool>> {
    Ok(get_opt_f64(row, name)?
        .is_some_and(|n| n != 0.0)
        .then_some(true))
}

/// A `sessions` row as `getPreviousSessions` maps it: NULL columns and unset
/// flags leave their field out.
fn row_to_session(row: &Row<'_>) -> Result<TerminalSession> {
    Ok(TerminalSession {
        id: get_text(row, "id")?,
        agent_type: AgentType(get_text(row, "agent_type")?),
        project_name: get_text(row, "project_name")?,
        project_path: get_text(row, "project_path")?,
        status: AgentStatus(get_text(row, "status")?),
        created_at: get_f64(row, "created_at")?,
        pid: get_f64(row, "pid")?,
        display_name: get_opt_text(row, "display_name")?,
        branch: get_opt_text(row, "branch")?,
        worktree_path: get_opt_text(row, "worktree_path")?,
        is_worktree: flag(row, "is_worktree")?,
        remote_host_id: get_opt_text(row, "remote_host_id")?,
        remote_host_label: get_opt_text(row, "remote_host_label")?,
        hook_session_id: get_opt_text(row, "hook_session_id")?,
        status_source: get_opt_text(row, "status_source")?,
        worktree_name: get_opt_text(row, "worktree_name")?,
        agent_session_id: get_opt_text(row, "agent_session_id")?,
        saved_at: get_opt_f64(row, "saved_at")?,
        shell_cwd: get_opt_text(row, "shell_cwd")?,
        head_commit: get_opt_text(row, "head_commit")?,
        renamed_by_person: flag(row, "renamed_by_person")?,
        group_id: get_opt_text(row, "group_id")?,
        cols: None,
        rows: None,
        shell_exit_code: None,
        rev: None,
    })
}

fn row_to_schedule_log_entry(row: &Row<'_>) -> Result<ScheduleLogEntry> {
    Ok(ScheduleLogEntry {
        workflow_id: get_text(row, "workflow_id")?,
        workflow_name: get_text(row, "workflow_name")?,
        executed_at: get_text(row, "executed_at")?,
        status: get_text(row, "status")?,
        sessions_launched: get_f64(row, "sessions_launched")?,
        error: get_opt_text(row, "error")?,
    })
}

/// A `session_events` row; `metadata` is its parsed JSON, left out when NULL.
fn row_to_session_event(row: &Row<'_>) -> Result<SessionEvent> {
    Ok(SessionEvent {
        id: Some(get_f64(row, "id")?),
        session_id: get_text(row, "session_id")?,
        event_type: SessionEventType(get_text(row, "event_type")?),
        timestamp: get_text(row, "timestamp")?,
        metadata: get_opt_text(row, "metadata")?
            .map(|text| parse_json(&text))
            .transpose()?,
    })
}

impl Store {
    // ---- Sessions ----------------------------------------------------------

    /// Replaces the saved sessions with `sessions`, in order. A record's own
    /// `savedAt` wins over now, so held sessions from an earlier run keep the
    /// age they had.
    pub fn save_sessions(&mut self, sessions: &[TerminalSession]) -> Result<()> {
        let saved_at = now_millis() as f64;
        let tx = self.write_transaction()?;
        tx.execute("DELETE FROM sessions", [])?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO sessions (id, agent_type, project_name, project_path, status, created_at, pid, display_name, branch, worktree_path, is_worktree, remote_host_id, remote_host_label, hook_session_id, status_source, saved_at, sort_order, worktree_name, agent_session_id, shell_cwd, head_commit, renamed_by_person, group_id)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for (i, s) in sessions.iter().enumerate() {
                insert.execute(params![
                    s.id,
                    s.agent_type.0,
                    s.project_name,
                    s.project_path,
                    s.status.0,
                    num(s.created_at),
                    num(s.pid),
                    s.display_name,
                    s.branch,
                    s.worktree_path,
                    i64::from(s.is_worktree == Some(true)),
                    s.remote_host_id,
                    s.remote_host_label,
                    s.hook_session_id,
                    s.status_source,
                    num(s.saved_at.unwrap_or(saved_at)),
                    i64::try_from(i).unwrap_or(i64::MAX),
                    s.worktree_name,
                    s.agent_session_id,
                    s.shell_cwd,
                    s.head_commit,
                    i64::from(s.renamed_by_person == Some(true)),
                    s.group_id,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_previous_sessions(&self) -> Result<Vec<TerminalSession>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT * FROM sessions ORDER BY sort_order ASC")?;
        collect(&mut stmt, [], row_to_session)
    }

    pub fn clear_sessions(&self) -> Result<()> {
        self.conn().execute("DELETE FROM sessions", [])?;
        Ok(())
    }

    // ---- Schedule log ------------------------------------------------------

    /// Appends an entry, then drops the oldest beyond the last 200.
    pub fn add_schedule_log_entry(&self, entry: &ScheduleLogEntry) -> Result<()> {
        let d = self.conn();
        d.execute(
            "INSERT INTO schedule_log (workflow_id, workflow_name, executed_at, status, sessions_launched, error)
     VALUES (?, ?, ?, ?, ?, ?)",
            params![
                entry.workflow_id,
                entry.workflow_name,
                entry.executed_at,
                entry.status,
                num(entry.sessions_launched),
                entry.error,
            ],
        )?;
        let count: i64 = d.query_row("SELECT COUNT(*) as c FROM schedule_log", [], |row| {
            row.get(0)
        })?;
        if count > MAX_LOG_ENTRIES {
            d.execute(
                "DELETE FROM schedule_log WHERE id IN (
        SELECT id FROM schedule_log ORDER BY id ASC LIMIT ?
      )",
                [count - MAX_LOG_ENTRIES],
            )?;
        }
        Ok(())
    }

    /// `if (workflowId)`: an empty string filters nothing.
    pub fn get_schedule_log_entries(
        &self,
        workflow_id: Option<&str>,
    ) -> Result<Vec<ScheduleLogEntry>> {
        match workflow_id.filter(|w| !w.is_empty()) {
            Some(workflow_id) => {
                let mut stmt = self
                    .conn()
                    .prepare("SELECT * FROM schedule_log WHERE workflow_id = ? ORDER BY id")?;
                collect(&mut stmt, [workflow_id], row_to_schedule_log_entry)
            }
            None => {
                let mut stmt = self
                    .conn()
                    .prepare("SELECT * FROM schedule_log ORDER BY id")?;
                collect(&mut stmt, [], row_to_schedule_log_entry)
            }
        }
    }

    pub fn clear_schedule_log(&self) -> Result<()> {
        self.conn().execute("DELETE FROM schedule_log", [])?;
        Ok(())
    }

    // ---- Effect receipts ---------------------------------------------------

    /// Records that the effect `effect_id` was acted on. True the first time,
    /// false for every delivery after: the caller acts only on true. `now`
    /// defaults to `Date.now()`.
    pub fn claim_effect(&self, effect_id: &str, kind: &str, now: Option<f64>) -> Result<bool> {
        let now = now.unwrap_or_else(|| now_millis() as f64);
        let changes = self.conn().execute(
            "INSERT OR IGNORE INTO effect_receipts (effect_id, kind, received_at) VALUES (?, ?, ?)",
            params![effect_id, kind, num(now)],
        )?;
        Ok(changes == 1)
    }

    /// Forgets receipts of `kind` older than `before`; answers how many went.
    pub fn prune_effect_receipts(&self, kind: &str, before: f64) -> Result<i64> {
        let changes = self.conn().execute(
            "DELETE FROM effect_receipts WHERE kind = ? AND received_at < ?",
            params![kind, num(before)],
        )?;
        Ok(i64::try_from(changes).unwrap_or(i64::MAX))
    }

    // ---- Session events ----------------------------------------------------

    /// Appends an event (metadata only when truthy), then keeps the newest
    /// 200 of its session.
    pub fn insert_session_event(&self, event: &SessionEvent) -> Result<()> {
        let d = self.conn();
        d.execute(
            "INSERT INTO session_events (session_id, event_type, timestamp, metadata)
     VALUES (?, ?, ?, ?)",
            params![
                event.session_id,
                event.event_type.0,
                event.timestamp,
                json_if_truthy(event.metadata.as_ref())?,
            ],
        )?;
        d.execute(
            "DELETE FROM session_events WHERE session_id = ? AND id NOT IN (
       SELECT id FROM session_events WHERE session_id = ? ORDER BY timestamp DESC LIMIT ?
     )",
            params![
                event.session_id,
                event.session_id,
                MAX_SESSION_EVENTS_PER_SESSION
            ],
        )?;
        Ok(())
    }

    /// Newest first. `if (eventType)`: an empty string filters nothing;
    /// `limit` defaults to 100.
    pub fn list_session_events(
        &self,
        event_type: Option<&str>,
        limit: Option<f64>,
    ) -> Result<Vec<SessionEvent>> {
        let limit = num(limit.unwrap_or(DEFAULT_SESSION_EVENT_LIMIT));
        match event_type.filter(|t| !t.is_empty()) {
            Some(event_type) => {
                let mut stmt = self.conn().prepare(
                    "SELECT * FROM session_events WHERE event_type = ? ORDER BY timestamp DESC LIMIT ?",
                )?;
                collect(&mut stmt, params![event_type, limit], row_to_session_event)
            }
            None => {
                let mut stmt = self
                    .conn()
                    .prepare("SELECT * FROM session_events ORDER BY timestamp DESC LIMIT ?")?;
                collect(&mut stmt, [limit], row_to_session_event)
            }
        }
    }

    /// Newest first; `limit` defaults to 100.
    pub fn list_session_events_by_session(
        &self,
        session_id: &str,
        limit: Option<f64>,
    ) -> Result<Vec<SessionEvent>> {
        let limit: SqlValue = num(limit.unwrap_or(DEFAULT_SESSION_EVENT_LIMIT));
        let mut stmt = self.conn().prepare(
            "SELECT * FROM session_events WHERE session_id = ? ORDER BY timestamp DESC LIMIT ?",
        )?;
        collect(&mut stmt, params![session_id, limit], row_to_session_event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use serde_json::json;

    fn session(id: &str, extra: serde_json::Value) -> TerminalSession {
        let mut value = json!({
            "id": id,
            "agentType": "claude",
            "projectName": "p",
            "projectPath": "/p",
            "status": "running",
            "createdAt": 1_700_000_000_000_i64,
            "pid": 42,
            "savedAt": null,
            "shellCwd": null,
            "groupId": null
        });
        if let (Some(base), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
            base.extend(extra.clone());
        }
        serde_json::from_value(value).unwrap()
    }

    fn keys(value: &impl serde::Serialize) -> Vec<String> {
        serde_json::to_value(value)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn sessions_round_trip_in_order_with_unset_fields_absent() {
        let mut store = test_support::store();
        let full = session(
            "b",
            json!({
                "displayName": "Build",
                "branch": "main",
                "worktreePath": "/w",
                "isWorktree": true,
                "remoteHostId": "h",
                "remoteHostLabel": "Host",
                "hookSessionId": "hk",
                "statusSource": "hook",
                "savedAt": 123,
                "worktreeName": "wt",
                "agentSessionId": "as",
                "shellCwd": "/cwd",
                "headCommit": "abc",
                "renamedByPerson": true,
                "groupId": "g",
                "cols": 80
            }),
        );
        let bare = session("a", json!({ "isWorktree": false }));
        let before = now_millis() as f64;
        store.save_sessions(&[full, bare]).unwrap();

        let loaded = store.get_previous_sessions().unwrap();
        assert_eq!(loaded.len(), 2);
        let (b, a) = (&loaded[0], &loaded[1]);
        assert_eq!(b.id, "b");
        assert_eq!(b.saved_at, Some(123.0));
        assert_eq!(b.is_worktree, Some(true));
        assert_eq!(b.renamed_by_person, Some(true));
        assert_eq!(b.group_id.as_deref(), Some("g"));
        assert_eq!(b.head_commit.as_deref(), Some("abc"));
        assert_eq!(b.cols, None);
        assert_eq!(b.created_at, 1_700_000_000_000.0);

        // The save stamps records that had no savedAt of their own.
        assert!(a.saved_at.is_some_and(|s| s >= before));
        let a_keys = keys(a);
        for absent in [
            "displayName",
            "branch",
            "isWorktree",
            "renamedByPerson",
            "statusSource",
            "headCommit",
        ] {
            assert!(!a_keys.contains(&absent.to_string()), "{a_keys:?}");
        }

        store.save_sessions(&[session("c", json!({}))]).unwrap();
        assert_eq!(store.get_previous_sessions().unwrap().len(), 1);
        store.clear_sessions().unwrap();
        assert!(store.get_previous_sessions().unwrap().is_empty());
    }

    fn log_entry(workflow_id: &str, n: usize) -> ScheduleLogEntry {
        ScheduleLogEntry {
            workflow_id: workflow_id.into(),
            workflow_name: "W".into(),
            executed_at: format!("t{n}"),
            status: "success".into(),
            sessions_launched: 1.0,
            error: None,
        }
    }

    #[test]
    fn schedule_log_trims_to_the_newest_entries() {
        let store = test_support::store();
        for n in 0..205 {
            let workflow = if n % 2 == 0 { "even" } else { "odd" };
            store
                .add_schedule_log_entry(&log_entry(workflow, n))
                .unwrap();
        }
        let all = store.get_schedule_log_entries(None).unwrap();
        assert_eq!(all.len(), 200);
        assert_eq!(all[0].executed_at, "t5");
        assert_eq!(all[199].executed_at, "t204");
        assert_eq!(store.get_schedule_log_entries(Some("")).unwrap().len(), 200);
        assert_eq!(
            store.get_schedule_log_entries(Some("odd")).unwrap().len(),
            100
        );
        assert!(!keys(&all[0]).contains(&"error".to_string()));

        let mut failed = log_entry("x", 0);
        failed.error = Some("nope".into());
        store.add_schedule_log_entry(&failed).unwrap();
        let x = store.get_schedule_log_entries(Some("x")).unwrap();
        assert_eq!(x[0].error.as_deref(), Some("nope"));

        store.clear_schedule_log().unwrap();
        assert!(store.get_schedule_log_entries(None).unwrap().is_empty());
    }

    #[test]
    fn effects_are_claimed_once_and_pruned_by_age() {
        let store = test_support::store();
        assert!(store.claim_effect("e1", "notify", Some(10.0)).unwrap());
        assert!(!store.claim_effect("e1", "notify", Some(20.0)).unwrap());
        assert!(store.claim_effect("e2", "notify", None).unwrap());
        assert!(store.claim_effect("e3", "other", Some(10.0)).unwrap());
        assert_eq!(store.prune_effect_receipts("notify", 11.0).unwrap(), 1);
        assert_eq!(store.prune_effect_receipts("notify", 11.0).unwrap(), 0);
        // Pruned, so the effect can be claimed again.
        assert!(store.claim_effect("e1", "notify", Some(30.0)).unwrap());
    }

    fn event(
        session_id: &str,
        event_type: &str,
        n: usize,
        metadata: Option<serde_json::Value>,
    ) -> SessionEvent {
        SessionEvent {
            id: None,
            session_id: session_id.into(),
            event_type: SessionEventType(event_type.into()),
            timestamp: format!("2026-10-05T00:00:{:02}.{:03}Z", n / 1000, n % 1000),
            metadata,
        }
    }

    #[test]
    fn session_events_keep_the_newest_per_session() {
        let store = test_support::store();
        for n in 0..203 {
            store
                .insert_session_event(&event("s1", "start", n, None))
                .unwrap();
        }
        store
            .insert_session_event(&event("s2", "exit", 0, Some(json!({ "code": 1 }))))
            .unwrap();
        store
            .insert_session_event(&event("s2", "start", 1, Some(json!(""))))
            .unwrap();

        let s1 = store
            .list_session_events_by_session("s1", Some(500.0))
            .unwrap();
        assert_eq!(s1.len(), 200);
        assert_eq!(s1[0].timestamp, event("", "", 202, None).timestamp);
        assert_eq!(s1[199].timestamp, event("", "", 3, None).timestamp);
        assert_eq!(
            store
                .list_session_events_by_session("s1", None)
                .unwrap()
                .len(),
            100
        );

        let s2 = store.list_session_events_by_session("s2", None).unwrap();
        assert_eq!(s2.len(), 2);
        // Falsy metadata is not stored, so it reads back absent.
        assert!(!keys(&s2[0]).contains(&"metadata".to_string()));
        assert_eq!(s2[1].metadata, Some(json!({ "code": 1 })));
        assert!(s2[1].id.is_some());

        let exits = store.list_session_events(Some("exit"), None).unwrap();
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0].session_id, "s2");
        assert_eq!(store.list_session_events(None, Some(5.0)).unwrap().len(), 5);
        assert_eq!(
            store.list_session_events(Some(""), None).unwrap().len(),
            100
        );
    }
}

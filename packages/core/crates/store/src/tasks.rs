//! Tasks: the targeted calls (`dbListTasks` and the rest). `loadConfig` and
//! `saveConfig` read and write the same rows through [`row_to_task`].

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, OptionalExtension, Row};
use serde_json::{Map, Value};
use vorn_protocol::{AiAgentType, TaskConfig, TaskStatus};

use crate::sql::{get_f64, get_opt_f64, get_opt_text, get_text, num};
use crate::{Result, Store};

/// A `tasks` row as `rowToTask` maps it: absent columns and NULLs leave the
/// field out, and `use_worktree` is there only when it is set.
pub(crate) fn row_to_task(row: &Row<'_>) -> Result<TaskConfig> {
    let use_worktree = get_opt_f64(row, "use_worktree")?.is_some_and(|n| n != 0.0);
    Ok(TaskConfig {
        id: get_text(row, "id")?,
        project_name: get_text(row, "project_name")?,
        title: get_text(row, "title")?,
        description: get_text(row, "description")?,
        status: TaskStatus(get_text(row, "status")?),
        order: get_f64(row, "order")?,
        assigned_session_id: get_opt_text(row, "assigned_session_id")?,
        assigned_agent: get_opt_text(row, "assigned_agent")?.map(AiAgentType),
        agent_session_id: get_opt_text(row, "agent_session_id")?,
        branch: get_opt_text(row, "branch")?,
        use_worktree: use_worktree.then_some(true),
        created_at: get_text(row, "created_at")?,
        updated_at: get_text(row, "updated_at")?,
        completed_at: get_opt_text(row, "completed_at")?,
        archived_at: get_opt_text(row, "archived_at")?,
        source_connector_id: get_opt_text(row, "source_connector_id")?,
        source_external_url: get_opt_text(row, "source_external_url")?,
        source_external_id: get_opt_text(row, "source_external_id")?,
        images: Vec::new(),
        worktree_path: None,
    })
}

impl Store {
    pub fn db_list_tasks(
        &self,
        project_name: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<TaskConfig>> {
        let mut sql = String::from("SELECT * FROM tasks");
        let mut clauses = Vec::new();
        let mut args = Vec::new();
        // `if (projectName)`: an empty string filters nothing.
        if let Some(project) = project_name.filter(|p| !p.is_empty()) {
            clauses.push("project_name = ?");
            args.push(project);
        }
        if let Some(status) = status.filter(|s| !s.is_empty()) {
            clauses.push("status = ?");
            args.push(status);
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(r#" ORDER BY "order""#);
        let mut stmt = self.conn().prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(args))?;
        let mut tasks = Vec::new();
        while let Some(row) = rows.next()? {
            tasks.push(row_to_task(row)?);
        }
        Ok(tasks)
    }

    pub fn db_get_task(&self, id: &str) -> Result<Option<TaskConfig>> {
        let mut stmt = self.conn().prepare("SELECT * FROM tasks WHERE id = ?")?;
        let mut rows = stmt.query([id])?;
        rows.next()?.map(row_to_task).transpose()
    }

    pub fn db_insert_task(&self, task: &TaskConfig) -> Result<()> {
        self.conn().execute(
            r#"INSERT INTO tasks (id, project_name, title, description, status, "order", assigned_session_id, assigned_agent, agent_session_id, branch, use_worktree, created_at, updated_at, completed_at, archived_at, source_connector_id, source_external_url, source_external_id)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            params![
                task.id,
                task.project_name,
                task.title,
                task.description,
                task.status.0,
                num(task.order),
                task.assigned_session_id,
                task.assigned_agent.as_ref().map(|a| a.0.as_str()),
                task.agent_session_id,
                task.branch,
                i64::from(task.use_worktree == Some(true)),
                task.created_at,
                task.updated_at,
                task.completed_at,
                task.archived_at,
                task.source_connector_id,
                task.source_external_url,
                task.source_external_id,
            ],
        )?;
        Ok(())
    }

    /// `updates` is the partial task as JSON. A field set to `undefined` in
    /// JavaScript does not reach here, which is the same as leaving it out for
    /// every field but `completedAt` and `archivedAt`, whose `in` test also
    /// counts an explicit `undefined`; `present` names the keys the caller's
    /// object had, so those two clear as they do there.
    pub fn db_update_task(
        &self,
        id: &str,
        updates: &Map<String, Value>,
        present: &[String],
    ) -> Result<()> {
        let mut sets: Vec<&str> = Vec::new();
        let mut args: Vec<SqlValue> = Vec::new();
        let mut set = |column: &'static str, value: SqlValue| {
            sets.push(column);
            args.push(value);
        };
        for (key, column) in [
            ("projectName", "project_name = ?"),
            ("title", "title = ?"),
            ("description", "description = ?"),
            ("status", "status = ?"),
            ("order", r#""order" = ?"#),
            ("branch", "branch = ?"),
        ] {
            if let Some(value) = updates.get(key) {
                set(column, bind(value));
            }
        }
        if let Some(value) = updates.get("useWorktree") {
            set(
                "use_worktree = ?",
                SqlValue::Integer(i64::from(crate::sql::truthy(value))),
            );
        }
        for (key, column) in [
            ("assignedAgent", "assigned_agent = ?"),
            ("assignedSessionId", "assigned_session_id = ?"),
            ("agentSessionId", "agent_session_id = ?"),
            ("updatedAt", "updated_at = ?"),
        ] {
            if let Some(value) = updates.get(key) {
                set(column, bind(value));
            }
        }
        for (key, column) in [
            ("completedAt", "completed_at = ?"),
            ("archivedAt", "archived_at = ?"),
        ] {
            if updates.contains_key(key) || present.iter().any(|k| k == key) {
                set(column, updates.get(key).map_or(SqlValue::Null, bind));
            }
        }
        for (key, column) in [
            ("sourceConnectorId", "source_connector_id = ?"),
            ("sourceExternalUrl", "source_external_url = ?"),
            ("sourceExternalId", "source_external_id = ?"),
        ] {
            if let Some(value) = updates.get(key) {
                set(column, bind(value));
            }
        }
        if sets.is_empty() {
            return Ok(());
        }
        args.push(SqlValue::Text(id.to_owned()));
        self.conn().execute(
            &format!("UPDATE tasks SET {} WHERE id = ?", sets.join(", ")),
            params_from_iter(args),
        )?;
        Ok(())
    }

    pub fn db_delete_task(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM tasks WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn db_get_max_task_order(&self, project_name: &str) -> Result<f64> {
        let max: Option<SqlValue> = self
            .conn()
            .query_row(
                r#"SELECT MAX("order") as m FROM tasks WHERE project_name = ?"#,
                [project_name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match max {
            Some(SqlValue::Integer(i)) => i as f64,
            Some(SqlValue::Real(f)) => f,
            _ => -1.0,
        })
    }

    /// Fallback lookup for orphan re-linking: a task whose own source ids
    /// match, even if its link row is gone.
    pub fn db_find_task_by_connector_external_id(
        &self,
        connector_id: &str,
        external_id: &str,
    ) -> Result<Option<TaskConfig>> {
        let id: Option<String> = self
            .conn()
            .query_row(
                "SELECT id FROM tasks WHERE source_connector_id = ? AND source_external_id = ? LIMIT 1",
                params![connector_id, external_id],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.db_get_task(&id),
            None => Ok(None),
        }
    }
}

/// A JSON value bound as libsql binds the JavaScript value: strings as text,
/// numbers as numbers, booleans as 1 or 0, null as NULL. Objects and arrays
/// never reach a column unserialized in the TypeScript store; they bind as
/// their JSON text here.
pub(crate) fn bind(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => SqlValue::Integer(i),
            None => n.as_f64().map_or(SqlValue::Null, num),
        },
        Value::String(s) => SqlValue::Text(s.clone()),
        Value::Array(_) | Value::Object(_) => SqlValue::Text(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support;
    use serde_json::json;

    fn task(id: &str, order: f64) -> vorn_protocol::TaskConfig {
        serde_json::from_value(json!({
            "id": id,
            "projectName": "p",
            "title": "T",
            "description": "",
            "status": "todo",
            "order": order,
            "createdAt": "2026-10-05T00:00:00.000Z",
            "updatedAt": "2026-10-05T00:00:00.000Z"
        }))
        .unwrap()
    }

    #[test]
    fn inserts_lists_and_updates_tasks() {
        let store = test_support::store();
        store.db_insert_task(&task("b", 2.0)).unwrap();
        store.db_insert_task(&task("a", 1.0)).unwrap();
        let ids: Vec<_> = store
            .db_list_tasks(Some("p"), None)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(store.db_get_max_task_order("p").unwrap(), 2.0);
        assert_eq!(store.db_get_max_task_order("none").unwrap(), -1.0);

        let updates = json!({ "status": "done", "completedAt": "x", "useWorktree": true });
        store
            .db_update_task("a", updates.as_object().unwrap(), &[])
            .unwrap();
        let a = store.db_get_task("a").unwrap().unwrap();
        assert_eq!(a.status.0, "done");
        assert_eq!(a.completed_at.as_deref(), Some("x"));
        assert_eq!(a.use_worktree, Some(true));

        // `{ completedAt: undefined }` clears it, as the `in` test does.
        store
            .db_update_task("a", &serde_json::Map::new(), &["completedAt".into()])
            .unwrap();
        assert_eq!(store.db_get_task("a").unwrap().unwrap().completed_at, None);
    }

    #[test]
    fn leaves_out_what_is_unset() {
        let store = test_support::store();
        store.db_insert_task(&task("a", 0.0)).unwrap();
        let json = serde_json::to_value(store.db_get_task("a").unwrap().unwrap()).unwrap();
        let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        assert!(!keys.contains(&"useWorktree".to_string()), "{keys:?}");
        assert!(!keys.contains(&"branch".to_string()), "{keys:?}");
    }
}

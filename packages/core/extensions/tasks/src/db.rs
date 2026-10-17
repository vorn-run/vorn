//! The board's tables as the writes see them: the same calls the core's
//! store answers (`dbGetTask`, `dbUpdateTask`, ...), so the rules in
//! [`crate::calls`] and [`crate::sources`] read the same in a test against
//! the store and in the component against Vorn.
//!
//! In the component ([`HostDb`]) a task is read through the core's
//! `task.get`, the one place a row becomes JSON; writes are held and run as
//! one storage transaction before the next read, and at the end of the call.

use serde_json::{json, Map, Value};
use vorn_extension_sdk::{Host, Statement};

use crate::js;

/// The store's calls the board's writes make.
pub trait Db {
    /// One call, as the core's store names it, with its positional args.
    fn call(&mut self, method: &str, args: Value) -> Result<Value, String>;
}

/// How many tasks one read of the core's list takes.
const PAGE: u64 = 1000;

/// The tables through Vorn's API.
pub struct HostDb<'h> {
    host: &'h mut dyn Host,
    pending: Vec<Statement>,
}

impl<'h> HostDb<'h> {
    pub fn new(host: &'h mut dyn Host) -> HostDb<'h> {
        HostDb {
            host,
            pending: Vec::new(),
        }
    }

    /// Runs the writes held so far as one transaction.
    pub fn flush(&mut self) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let statements = std::mem::take(&mut self.pending);
        self.host
            .transaction(&statements)
            .map(drop)
            .map_err(|e| e.message)
    }

    fn query(&mut self, sql: &str, params: Vec<Value>) -> Result<Vec<Value>, String> {
        self.flush()?;
        self.host.query(sql, &params).map_err(|e| e.message)
    }

    fn write(&mut self, sql: impl Into<String>, params: Vec<Value>) {
        self.pending.push(Statement::new(sql, params));
    }

    fn task(&mut self, id: &Value) -> Result<Value, String> {
        self.flush()?;
        // A task id is text; any other id names no task, as the store's string lookup finds none.
        let Some(id) = id.as_str() else {
            return Ok(Value::Null);
        };
        self.host
            .call("task.get", json!({ "id": id }))
            .map_err(|e| e.message)
    }

    fn tasks(&mut self, project: Option<&str>, status: Option<&str>) -> Result<Value, String> {
        self.flush()?;
        let mut all = Vec::new();
        let mut offset = Value::from(0);
        while !offset.is_null() {
            let mut params = json!({ "includeDescription": true, "offset": offset, "limit": PAGE });
            if let Some(p) = project {
                params["projectName"] = json!(p);
            }
            if let Some(s) = status {
                params["status"] = json!(s);
            }
            let page = self
                .host
                .call("task.list", params)
                .map_err(|e| e.message)?;
            all.extend(page["tasks"].as_array().cloned().unwrap_or_default());
            offset = page.get("next").cloned().unwrap_or(Value::Null);
        }
        Ok(Value::Array(all))
    }

    fn link(&mut self, sql: &str, params: Vec<Value>) -> Result<Value, String> {
        Ok(self
            .query(sql, params)?
            .into_iter()
            .next()
            .map_or(Value::Null, camel))
    }
}

/// `args[i]`, null when it is not there.
fn at(args: &Value, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Null)
}

fn text(args: &Value, i: usize) -> Option<String> {
    args.get(i).and_then(Value::as_str).map(str::to_owned)
}

/// A value bound as the store binds one: whole numbers as integers.
fn bind(v: &Value) -> Value {
    match v {
        Value::Number(n) => n.as_f64().map_or(Value::Null, js::num),
        other => other.clone(),
    }
}

/// A row's `snake_case` columns as `camelCase` keys.
fn camel(row: Value) -> Value {
    let Value::Object(o) = row else {
        return row;
    };
    let mut out = Map::with_capacity(o.len());
    for (k, v) in o {
        let mut key = String::with_capacity(k.len());
        let mut up = false;
        for c in k.chars() {
            if c == '_' {
                up = true;
            } else if up {
                key.push(c.to_ascii_uppercase());
                up = false;
            } else {
                key.push(c);
            }
        }
        out.insert(key, v);
    }
    Value::Object(out)
}

/// `UPDATE tasks` for `dbUpdateTask(id, updates, present)`: a missing field
/// is left as it is, but `completedAt` and `archivedAt` clear when `present`
/// names them without a value.
fn update_task(updates: &Map<String, Value>, present: &[String]) -> (Vec<&'static str>, Vec<Value>) {
    let mut sets = Vec::new();
    let mut args = Vec::new();
    for (key, column) in [
        ("projectName", "project_name = ?"),
        ("title", "title = ?"),
        ("description", "description = ?"),
        ("status", "status = ?"),
        ("order", r#""order" = ?"#),
        ("branch", "branch = ?"),
    ] {
        if let Some(v) = updates.get(key) {
            sets.push(column);
            args.push(bind(v));
        }
    }
    if let Some(v) = updates.get("useWorktree") {
        sets.push("use_worktree = ?");
        args.push(json!(i64::from(js::truthy(v))));
    }
    for (key, column) in [
        ("assignedAgent", "assigned_agent = ?"),
        ("assignedSessionId", "assigned_session_id = ?"),
        ("agentSessionId", "agent_session_id = ?"),
        ("updatedAt", "updated_at = ?"),
    ] {
        if let Some(v) = updates.get(key) {
            sets.push(column);
            args.push(bind(v));
        }
    }
    for (key, column) in [
        ("completedAt", "completed_at = ?"),
        ("archivedAt", "archived_at = ?"),
    ] {
        if updates.contains_key(key) || present.iter().any(|k| k == key) {
            sets.push(column);
            args.push(updates.get(key).map_or(Value::Null, bind));
        }
    }
    for (key, column) in [
        ("sourceConnectorId", "source_connector_id = ?"),
        ("sourceExternalUrl", "source_external_url = ?"),
        ("sourceExternalId", "source_external_id = ?"),
    ] {
        if let Some(v) = updates.get(key) {
            sets.push(column);
            args.push(bind(v));
        }
    }
    (sets, args)
}

/// `INSERT INTO tasks` for a task as the store takes one.
fn insert_task(t: &Value) -> Vec<Value> {
    let field = |k: &str| t.get(k).map_or(Value::Null, bind);
    let or_empty = |k: &str| match t.get(k) {
        None | Some(Value::Null) => json!(""),
        Some(v) => bind(v),
    };
    vec![
        field("id"),
        field("projectName"),
        field("title"),
        or_empty("description"),
        t.get("status").filter(|s| !s.is_null()).map_or(json!("todo"), bind),
        t.get("order").map_or(json!(0), bind),
        field("assignedSessionId"),
        field("assignedAgent"),
        field("agentSessionId"),
        field("branch"),
        json!(i64::from(t.get("useWorktree") == Some(&Value::Bool(true)))),
        field("createdAt"),
        field("updatedAt"),
        field("completedAt"),
        field("archivedAt"),
        field("sourceConnectorId"),
        field("sourceExternalUrl"),
        field("sourceExternalId"),
    ]
}

impl Db for HostDb<'_> {
    fn call(&mut self, method: &str, args: Value) -> Result<Value, String> {
        match method {
            "dbGetTask" => self.task(&at(&args, 0)),
            "dbListTasks" => {
                let (project, status) = (text(&args, 0), text(&args, 1));
                self.tasks(
                    project.as_deref().filter(|p| !p.is_empty()),
                    status.as_deref().filter(|s| !s.is_empty()),
                )
            }
            "dbGetProject" => Ok(self
                .query(
                    "SELECT name FROM projects WHERE name = ?",
                    vec![at(&args, 0)],
                )?
                .into_iter()
                .next()
                .unwrap_or(Value::Null)),
            "dbGetMaxTaskOrder" => {
                let rows = self.query(
                    r#"SELECT MAX("order") AS m FROM tasks WHERE project_name = ?"#,
                    vec![at(&args, 0)],
                )?;
                Ok(rows
                    .first()
                    .and_then(|r| r.get("m"))
                    .filter(|m| m.is_number())
                    .cloned()
                    .unwrap_or(json!(-1)))
            }
            "dbInsertTask" => {
                self.write(
                    r#"INSERT INTO tasks (id, project_name, title, description, status, "order", assigned_session_id, assigned_agent, agent_session_id, branch, use_worktree, created_at, updated_at, completed_at, archived_at, source_connector_id, source_external_url, source_external_id)
                       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
                    insert_task(&at(&args, 0)),
                );
                Ok(Value::Null)
            }
            "dbUpdateTask" => {
                let updates = at(&args, 1).as_object().cloned().unwrap_or_default();
                let present: Vec<String> = at(&args, 2)
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|k| k.as_str().map(str::to_owned))
                    .collect();
                let (sets, mut params) = update_task(&updates, &present);
                if !sets.is_empty() {
                    params.push(at(&args, 0));
                    self.write(
                        format!("UPDATE tasks SET {} WHERE id = ?", sets.join(", ")),
                        params,
                    );
                }
                Ok(Value::Null)
            }
            "dbDeleteTask" => {
                self.write("DELETE FROM tasks WHERE id = ?", vec![at(&args, 0)]);
                Ok(Value::Null)
            }
            "dbFindTaskByConnectorExternalId" => {
                let found = self.query(
                    "SELECT id FROM tasks WHERE source_connector_id = ? AND source_external_id = ? LIMIT 1",
                    vec![at(&args, 0), at(&args, 1)],
                )?;
                match found.first().and_then(|r| r.get("id")).cloned() {
                    Some(id) => self.task(&id),
                    None => Ok(Value::Null),
                }
            }
            "dbGetTaskSourceLink" => self.link(
                "SELECT * FROM task_source_links WHERE task_id = ?",
                vec![at(&args, 0)],
            ),
            "dbGetTaskSourceLinkByExternalId" => self.link(
                "SELECT * FROM task_source_links WHERE connection_id = ? AND external_id = ?",
                vec![at(&args, 0), at(&args, 1)],
            ),
            "dbInsertTaskSourceLink" => {
                let l = at(&args, 0);
                let f = |k: &str| l.get(k).map_or(Value::Null, bind);
                self.write(
                    "INSERT INTO task_source_links (task_id, connection_id, connector_id, external_id, external_url, source_status_raw, source_updated_at, last_synced_at, conflict_state)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    vec![
                        f("taskId"),
                        f("connectionId"),
                        f("connectorId"),
                        f("externalId"),
                        f("externalUrl"),
                        f("sourceStatusRaw"),
                        f("sourceUpdatedAt"),
                        f("lastSyncedAt"),
                        f("conflictState"),
                    ],
                );
                Ok(Value::Null)
            }
            "dbUpdateTaskSourceLink" => {
                let updates = at(&args, 1);
                let mut sets = Vec::new();
                let mut params = Vec::new();
                for (key, column) in [
                    ("sourceStatusRaw", "source_status_raw = ?"),
                    ("sourceUpdatedAt", "source_updated_at = ?"),
                    ("lastSyncedAt", "last_synced_at = ?"),
                    ("conflictState", "conflict_state = ?"),
                ] {
                    if let Some(v) = updates.get(key) {
                        sets.push(column);
                        params.push(bind(v));
                    }
                }
                if !sets.is_empty() {
                    params.push(at(&args, 0));
                    self.write(
                        format!(
                            "UPDATE task_source_links SET {} WHERE task_id = ?",
                            sets.join(", ")
                        ),
                        params,
                    );
                }
                Ok(Value::Null)
            }
            other => Err(format!("the board makes no {other} call")),
        }
    }
}

impl Drop for HostDb<'_> {
    /// Writes held when a call ends early are run all the same, as the store ran each at once.
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_read_back_as_the_store_names_them() {
        assert_eq!(
            camel(json!({ "task_id": "t", "source_status_raw": "open", "x": 1 })),
            json!({ "taskId": "t", "sourceStatusRaw": "open", "x": 1 })
        );
    }

    #[test]
    fn an_update_sets_what_it_names_and_clears_the_dates_it_lists() {
        let updates = json!({ "title": "T", "order": 2.0, "useWorktree": "yes", "completedAt": null });
        let (sets, args) = update_task(updates.as_object().unwrap(), &["archivedAt".into()]);
        assert_eq!(
            sets,
            [
                "title = ?",
                r#""order" = ?"#,
                "use_worktree = ?",
                "completed_at = ?",
                "archived_at = ?"
            ]
        );
        assert_eq!(args, [json!("T"), json!(2), json!(1), Value::Null, Value::Null]);
    }
}

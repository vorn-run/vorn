//! Workflow runs and the state of each node in them (`saveWorkflowRun`,
//! `listWorkflowRuns` and the rest).
//!
//! Reads return JSON objects rather than [`WorkflowExecution`]: the lists add
//! `workflowName`, which the type does not have, and building the object from
//! the columns keeps numbers as libsql hands them to JavaScript.

use std::collections::HashMap;

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, Row};
use serde_json::{Map, Value};
use vorn_protocol::{NodeExecutionState, WorkflowExecution};

use crate::sql::{column_value, get_text, json_if_truthy, json_text, num, opt_num};
use crate::{Result, Store};

/// Runs kept per workflow; older finished ones are trimmed on save.
const MAX_WORKFLOW_RUNS: i64 = 50;

/// `workflowRunId`: the run's own id, or one made from its workflow and start.
fn workflow_run_id(execution: &WorkflowExecution) -> String {
    if execution.run_id.is_empty() {
        format!("{}:{}", execution.workflow_id, execution.started_at)
    } else {
        execution.run_id.clone()
    }
}

/// `ns.feedback?.length ? JSON.stringify(ns.feedback) : null`.
fn feedback_text(feedback: Option<&Value>) -> Result<SqlValue> {
    let stored = match feedback {
        Some(v @ Value::Array(items)) if !items.is_empty() => Some(v),
        Some(v @ Value::String(s)) if !s.is_empty() => Some(v),
        _ => None,
    };
    match stored {
        Some(v) => Ok(SqlValue::Text(json_text(v)?)),
        None => Ok(SqlValue::Null),
    }
}

/// `parseRunInputs` and `parseStructuredOutput`: a JSON object, or nothing
/// for empty text, a parse error, an array or a scalar.
fn parse_object(raw: Option<&str>) -> Option<Value> {
    let raw = raw.filter(|r| !r.is_empty())?;
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(Value::is_object)
}

/// `parseGateFeedback`: a non-empty array, or nothing.
fn parse_feedback(raw: Option<&str>) -> Option<Value> {
    serde_json::from_str::<Value>(raw?)
        .ok()
        .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
}

/// Sets `key` to the column as JavaScript reads it, unless it is NULL
/// (`...(r.col != null && { key: r.col })`).
fn put_if_set(obj: &mut Map<String, Value>, key: &str, row: &Row<'_>, column: &str) -> Result<()> {
    let value = column_value(row, column)?;
    if !value.is_null() {
        obj.insert(key.to_owned(), value);
    }
    Ok(())
}

/// A `workflow_run_nodes` row as `mapNodeRow` maps it.
fn map_node_row(row: &Row<'_>) -> Result<Value> {
    let mut node = Map::new();
    node.insert("nodeId".into(), column_value(row, "node_id")?);
    node.insert("status".into(), column_value(row, "status")?);
    for (key, column) in [
        ("startedAt", "started_at"),
        ("completedAt", "completed_at"),
        ("sessionId", "session_id"),
        ("error", "error"),
        ("logs", "logs"),
        ("taskId", "task_id"),
        ("agentSessionId", "agent_session_id"),
        ("agentType", "agent_type"),
        ("projectName", "project_name"),
        ("projectPath", "project_path"),
        ("approvedAt", "approved_at"),
        ("diagnostics", "diagnostics"),
        ("output", "output"),
    ] {
        put_if_set(&mut node, key, row, column)?;
    }
    // `parseStructuredOutput` runs on any non-NULL column, but an empty
    // string fails `JSON.parse` there just as `parse_object` skips it.
    if let Some(structured) = parse_object(text_column(row, "structured_output")?.as_deref()) {
        node.insert("structuredOutput".into(), structured);
    }
    put_if_set(&mut node, "iteration", row, "iteration")?;
    put_if_set(&mut node, "worktreePath", row, "worktree_path")?;
    put_if_set(&mut node, "worktreeName", row, "worktree_name")?;
    if let Some(origin) = text_column(row, "worktree_origin")? {
        if origin == "created" || origin == "inherited" {
            node.insert("worktreeOrigin".into(), Value::String(origin));
        }
    }
    if text_column(row, "waiting_for")?.as_deref() == Some("signIn") {
        node.insert("waitingFor".into(), Value::from("signIn"));
    }
    put_if_set(&mut node, "message", row, "message")?;
    put_if_set(&mut node, "viewToken", row, "view_token")?;
    put_if_set(&mut node, "round", row, "round")?;
    if let Some(feedback) = parse_feedback(text_column(row, "feedback")?.as_deref()) {
        node.insert("feedback".into(), feedback);
    }
    put_if_set(&mut node, "rejectedAt", row, "rejected_at")?;
    put_if_set(&mut node, "editableText", row, "editable_text")?;
    put_if_set(&mut node, "editedText", row, "edited_text")?;
    Ok(Value::Object(node))
}

/// A column only when it holds text: the strict `=== 'x'` tests of the
/// mappers never match a number, and `JSON.parse` of a number is not an
/// object or an array either.
fn text_column(row: &Row<'_>, column: &str) -> Result<Option<String>> {
    Ok(match row.get_ref(column)? {
        rusqlite::types::ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        _ => None,
    })
}

/// `fetchNodesByRunIds`: every node of these runs in one query, grouped by
/// run in the order SQLite returns them.
fn fetch_nodes_by_run_ids(
    conn: &Connection,
    run_ids: &[String],
) -> Result<HashMap<String, Vec<Value>>> {
    let mut out: HashMap<String, Vec<Value>> = HashMap::new();
    if run_ids.is_empty() {
        return Ok(out);
    }
    let placeholders = vec!["?"; run_ids.len()].join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM workflow_run_nodes WHERE run_id IN ({placeholders})"
    ))?;
    let mut rows = stmt.query(params_from_iter(run_ids))?;
    while let Some(row) = rows.next()? {
        let run_id = get_text(row, "run_id")?;
        out.entry(run_id).or_default().push(map_node_row(row)?);
    }
    Ok(out)
}

/// A `workflow_runs` row as `mapRunRows` maps it, without its nodes.
fn map_run_row(row: &Row<'_>) -> Result<(String, Map<String, Value>)> {
    let id = get_text(row, "id")?;
    let mut run = Map::new();
    run.insert("runId".into(), column_value(row, "id")?);
    run.insert("workflowId".into(), column_value(row, "workflow_id")?);
    run.insert("startedAt".into(), column_value(row, "started_at")?);
    put_if_set(&mut run, "completedAt", row, "completed_at")?;
    run.insert("status".into(), column_value(row, "status")?);
    put_if_set(&mut run, "triggerTaskId", row, "trigger_task_id")?;
    if let Some(inputs) = parse_object(text_column(row, "inputs")?.as_deref()) {
        run.insert("inputs".into(), inputs);
    }
    if let Some(item) = parse_object(text_column(row, "connector_item")?.as_deref()) {
        run.insert("connectorItem".into(), item);
    }
    put_if_set(&mut run, "connectorInboxId", row, "connector_inbox_id")?;
    put_if_set(
        &mut run,
        "connectorInboxLeaseToken",
        row,
        "connector_inbox_lease_token",
    )?;
    if let Some(disposition) = text_column(row, "connector_inbox_disposition")? {
        if disposition == "processed" || disposition == "retry" {
            run.insert(
                "connectorInboxDisposition".into(),
                Value::String(disposition),
            );
        }
    }
    // Only the queries that join `workflows` have the column.
    if row.as_ref().column_index("workflow_name").is_ok() {
        put_if_set(&mut run, "workflowName", row, "workflow_name")?;
    }
    if let Some(definition) = parse_object(text_column(row, "definition")?.as_deref()) {
        run.insert("definition".into(), definition);
    }
    Ok((id, run))
}

/// Runs a query for run rows and attaches their nodes, as each list does
/// with `mapRunRows` over `fetchNodesByRunIds`.
fn query_runs(conn: &Connection, sql: &str, args: &[SqlValue]) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query(params_from_iter(args))?;
    let mut runs = Vec::new();
    while let Some(row) = rows.next()? {
        runs.push(map_run_row(row)?);
    }
    let ids: Vec<String> = runs.iter().map(|(id, _)| id.clone()).collect();
    let mut nodes = fetch_nodes_by_run_ids(conn, &ids)?;
    Ok(runs
        .into_iter()
        .map(|(id, mut run)| {
            let node_states = nodes.remove(&id).unwrap_or_default();
            run.insert("nodeStates".into(), Value::Array(node_states));
            Value::Object(run)
        })
        .collect())
}

/// Binds the columns of one node as `saveWorkflowRun` does.
fn node_params(run_id: &str, ns: &NodeExecutionState) -> Result<Vec<SqlValue>> {
    let text = |s: &Option<String>| s.clone().map_or(SqlValue::Null, SqlValue::Text);
    Ok(vec![
        SqlValue::Text(run_id.to_owned()),
        SqlValue::Text(ns.node_id.clone()),
        SqlValue::Text(ns.status.0.clone()),
        text(&ns.started_at),
        text(&ns.completed_at),
        text(&ns.session_id),
        text(&ns.error),
        text(&ns.logs),
        text(&ns.task_id),
        text(&ns.agent_session_id),
        text(&ns.agent_type.as_ref().map(|a| a.0.clone())),
        text(&ns.project_name),
        text(&ns.project_path),
        text(&ns.approved_at),
        text(&ns.diagnostics),
        text(&ns.output),
        json_if_truthy(ns.structured_output.as_ref())?,
        opt_num(ns.iteration),
        text(&ns.worktree_path),
        text(&ns.worktree_name),
        text(&ns.worktree_origin),
        text(&ns.waiting_for),
        text(&ns.message),
        text(&ns.view_token),
        opt_num(ns.round),
        feedback_text(ns.feedback.as_ref())?,
        text(&ns.rejected_at),
        text(&ns.editable_text),
        text(&ns.edited_text),
    ])
}

impl Store {
    /// Upserts the run and replaces its nodes, then trims the workflow's
    /// oldest finished runs past [`MAX_WORKFLOW_RUNS`]. Returns the trimmed
    /// run ids, whose gate views the caller removes.
    pub fn save_workflow_run(&mut self, execution: &WorkflowExecution) -> Result<Vec<String>> {
        let run_id = workflow_run_id(execution);
        let tx = self.write_transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO workflow_runs (
         id, workflow_id, started_at, completed_at, status, trigger_task_id,
         inputs, connector_item, connector_inbox_id, connector_inbox_lease_token,
         connector_inbox_disposition, definition
       ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                run_id,
                execution.workflow_id,
                execution.started_at,
                execution.completed_at,
                execution.status,
                execution.trigger_task_id,
                json_if_truthy(execution.inputs.as_ref())?,
                json_if_truthy(execution.connector_item.as_ref())?,
                opt_num(execution.connector_inbox_id),
                execution.connector_inbox_lease_token,
                execution.connector_inbox_disposition,
                json_if_truthy(execution.definition.as_ref())?,
            ],
        )?;

        tx.execute("DELETE FROM workflow_run_nodes WHERE run_id = ?", [&run_id])?;
        {
            let mut insert_node = tx.prepare(
                "INSERT INTO workflow_run_nodes (run_id, node_id, status, started_at, completed_at, session_id, error, logs, task_id, agent_session_id, agent_type, project_name, project_path, approved_at, diagnostics, output, structured_output, iteration, worktree_path, worktree_name, worktree_origin, waiting_for, message, view_token, round, feedback, rejected_at, editable_text, edited_text)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for ns in &execution.node_states {
                insert_node.execute(params_from_iter(node_params(&run_id, ns)?))?;
            }
        }

        // Active runs, and connector runs whose inbox row is still unprocessed,
        // stay for restart recovery.
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) as c FROM workflow_runs WHERE workflow_id = ?",
            [&execution.workflow_id],
            |row| row.get(0),
        )?;
        let mut trimmed = Vec::new();
        if count > MAX_WORKFLOW_RUNS {
            {
                let mut stmt = tx.prepare(
                    "SELECT id FROM workflow_runs
            WHERE workflow_id = ?
              AND status != 'running'
              AND (
                connector_inbox_id IS NULL
                OR NOT EXISTS (
                  SELECT 1 FROM connector_inbox
                  WHERE connector_inbox.id = workflow_runs.connector_inbox_id
                    AND connector_inbox.status != 'processed'
                )
              )
            ORDER BY started_at ASC
            LIMIT ?",
                )?;
                let mut rows =
                    stmt.query(params![execution.workflow_id, count - MAX_WORKFLOW_RUNS])?;
                while let Some(row) = rows.next()? {
                    trimmed.push(get_text(row, "id")?);
                }
            }
            let mut remove = tx.prepare("DELETE FROM workflow_runs WHERE id = ?")?;
            for id in &trimmed {
                remove.execute([id])?;
            }
        }
        tx.commit()?;
        Ok(trimmed)
    }

    /// Every run id kept, so review pages of runs trimmed while the server
    /// was down can go too.
    pub fn list_workflow_run_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare("SELECT id FROM workflow_runs")?;
        let mut rows = stmt.query([])?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next()? {
            ids.push(get_text(row, "id")?);
        }
        Ok(ids)
    }

    /// The newest run fed by this connector inbox row.
    pub fn db_get_workflow_run_by_connector_inbox_id(
        &self,
        connector_inbox_id: i64,
    ) -> Result<Option<Value>> {
        let runs = query_runs(
            self.conn(),
            "SELECT * FROM workflow_runs
       WHERE connector_inbox_id = ?
       ORDER BY started_at DESC
       LIMIT 1",
            &[SqlValue::Integer(connector_inbox_id)],
        )?;
        Ok(runs.into_iter().next())
    }

    /// One run by its id, as the engine reads it when a gate is answered
    /// after a restart.
    pub fn get_workflow_run(&self, run_id: &str) -> Result<Option<Value>> {
        let runs = query_runs(
            self.conn(),
            "SELECT * FROM workflow_runs WHERE id = ?",
            &[SqlValue::Text(run_id.to_owned())],
        )?;
        Ok(runs.into_iter().next())
    }

    /// A workflow's runs, newest first; `limit` defaults to 20.
    pub fn list_workflow_runs(&self, workflow_id: &str, limit: Option<f64>) -> Result<Vec<Value>> {
        query_runs(
            self.conn(),
            "SELECT * FROM workflow_runs WHERE workflow_id = ? ORDER BY started_at DESC LIMIT ?",
            &[
                SqlValue::Text(workflow_id.to_owned()),
                num(limit.unwrap_or(20.0)),
            ],
        )
    }

    /// Runs a task triggered or has a node in, with their workflow's name;
    /// `limit` defaults to 20.
    pub fn list_workflow_runs_by_task(
        &self,
        task_id: &str,
        limit: Option<f64>,
    ) -> Result<Vec<Value>> {
        query_runs(
            self.conn(),
            "SELECT DISTINCT wr.*, w.name as workflow_name
       FROM workflow_runs wr
       LEFT JOIN workflows w ON w.id = wr.workflow_id
       WHERE wr.trigger_task_id = ?
          OR wr.id IN (SELECT run_id FROM workflow_run_nodes WHERE task_id = ?)
       ORDER BY wr.started_at DESC
       LIMIT ?",
            &[
                SqlValue::Text(task_id.to_owned()),
                SqlValue::Text(task_id.to_owned()),
                num(limit.unwrap_or(20.0)),
            ],
        )
    }

    /// Runs still `running`, which the reconciler closes out at startup.
    pub fn list_running_runs(&self) -> Result<Vec<Value>> {
        query_runs(
            self.conn(),
            "SELECT * FROM workflow_runs
       WHERE status = 'running'
       ORDER BY started_at DESC",
            &[],
        )
    }

    /// Every run with a waiting node; `Some("signIn")` keeps only those
    /// waiting for a sign-in. No limit, so the badge matches the backlog.
    pub fn list_runs_with_waiting_gates(&self, kind: Option<&str>) -> Result<Vec<Value>> {
        let filter = if kind == Some("signIn") {
            " AND wrn.waiting_for = 'signIn'"
        } else {
            ""
        };
        query_runs(
            self.conn(),
            &format!(
                "SELECT DISTINCT wr.*
       FROM workflow_runs wr
       JOIN workflow_run_nodes wrn ON wrn.run_id = wr.id
       WHERE wrn.status = 'waiting'{filter}
       ORDER BY wr.started_at DESC"
            ),
            &[],
        )
    }

    /// Run history across workflows with each workflow's name, narrowed to
    /// a workspace when one is given (which leaves out runs of deleted
    /// workflows). `limit` defaults to 50 and is clamped to 1..=500.
    pub fn list_all_workflow_runs(
        &self,
        workspace_id: Option<&str>,
        limit: Option<f64>,
    ) -> Result<Vec<Value>> {
        // `Math.max(1, Math.min(limit, 500))`, NaN staying NaN as there.
        let capped = num(limit.unwrap_or(50.0).clamp(1.0, 500.0));
        let workspace_id = workspace_id.filter(|w| !w.is_empty());
        let filter = if workspace_id.is_some() {
            "WHERE w.id IS NOT NULL AND COALESCE(w.workspace_id, 'personal') = ?"
        } else {
            ""
        };
        let sql = format!(
            "SELECT wr.*, w.name as workflow_name
               FROM workflow_runs wr
               LEFT JOIN workflows w ON w.id = wr.workflow_id
               {filter}
               ORDER BY wr.started_at DESC
               LIMIT ?"
        );
        let args = match workspace_id {
            Some(w) => vec![SqlValue::Text(w.to_owned()), capped],
            None => vec![capped],
        };
        query_runs(self.conn(), &sql, &args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use serde_json::json;

    fn run(workflow_id: &str, run_id: &str, started_at: &str, status: &str) -> WorkflowExecution {
        serde_json::from_value(json!({
            "runId": run_id,
            "workflowId": workflow_id,
            "startedAt": started_at,
            "status": status,
            "nodeStates": []
        }))
        .unwrap()
    }

    fn node(value: Value) -> NodeExecutionState {
        serde_json::from_value(value).unwrap()
    }

    fn add_workflow(store: &Store, id: &str, workspace: &str) {
        store
            .conn()
            .execute(
                "INSERT INTO workflows (id, name, icon, icon_color, workspace_id) VALUES (?, ?, 'x', '#000', ?)",
                params![id, format!("Flow {id}"), workspace],
            )
            .unwrap();
    }

    #[test]
    fn saves_and_reads_back_a_run_with_its_nodes() {
        let mut store = test_support::store();
        let mut exec = run("wf", "", "2026-10-05T00:00:00.000Z", "running");
        exec.inputs = Some(json!({ "a": 1 }));
        exec.connector_item = Some(json!([1, 2]));
        exec.connector_inbox_id = Some(7.0);
        exec.connector_inbox_disposition = Some("bogus".into());
        exec.definition = Some(json!({ "id": "wf" }));
        exec.node_states = vec![
            node(json!({
                "nodeId": "n1",
                "status": "completed",
                "structuredOutput": { "ok": true },
                "feedback": [{ "text": "hi" }],
                "worktreeOrigin": "elsewhere",
                "iteration": 2,
                "waitingFor": "signIn"
            })),
            node(
                json!({ "nodeId": "n2", "status": "waiting", "feedback": [], "worktreeOrigin": "created" }),
            ),
        ];
        assert!(store.save_workflow_run(&exec).unwrap().is_empty());

        let id = "wf:2026-10-05T00:00:00.000Z";
        let got = store.get_workflow_run(id).unwrap().unwrap();
        assert_eq!(
            got,
            json!({
                "runId": id,
                "workflowId": "wf",
                "startedAt": "2026-10-05T00:00:00.000Z",
                "status": "running",
                "inputs": { "a": 1 },
                "connectorInboxId": 7,
                "definition": { "id": "wf" },
                "nodeStates": [
                    {
                        "nodeId": "n1",
                        "status": "completed",
                        "structuredOutput": { "ok": true },
                        "iteration": 2,
                        "waitingFor": "signIn",
                        "feedback": [{ "text": "hi" }]
                    },
                    { "nodeId": "n2", "status": "waiting", "worktreeOrigin": "created" }
                ]
            })
        );
        let keys: Vec<_> = got["nodeStates"][0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            keys,
            [
                "nodeId",
                "status",
                "structuredOutput",
                "iteration",
                "waitingFor",
                "feedback"
            ]
        );
        assert_eq!(
            store.db_get_workflow_run_by_connector_inbox_id(7).unwrap(),
            Some(got)
        );
        assert_eq!(
            store.db_get_workflow_run_by_connector_inbox_id(8).unwrap(),
            None
        );
        assert_eq!(store.get_workflow_run("nope").unwrap(), None);
        assert_eq!(store.list_workflow_run_ids().unwrap(), [id]);
    }

    #[test]
    fn saving_again_replaces_the_node_rows() {
        let mut store = test_support::store();
        let mut exec = run("wf", "r1", "2026-10-05T00:00:00.000Z", "running");
        exec.node_states = vec![node(json!({ "nodeId": "a", "status": "running" }))];
        store.save_workflow_run(&exec).unwrap();
        exec.status = "completed".into();
        exec.node_states = vec![
            node(json!({ "nodeId": "b", "status": "completed" })),
            node(json!({ "nodeId": "c", "status": "completed" })),
        ];
        store.save_workflow_run(&exec).unwrap();
        let got = store.get_workflow_run("r1").unwrap().unwrap();
        assert_eq!(got["status"], "completed");
        let nodes: Vec<_> = got["nodeStates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["nodeId"].clone())
            .collect();
        assert_eq!(nodes, [json!("b"), json!("c")]);
        assert_eq!(store.list_workflow_runs("wf", None).unwrap().len(), 1);
    }

    #[test]
    fn trims_finished_runs_past_the_cap_but_keeps_running_ones() {
        let mut store = test_support::store();
        // The two oldest are running and survive; the next oldest finished ones go.
        for i in 0..52 {
            let status = if i < 2 { "running" } else { "completed" };
            let exec = run(
                "wf",
                &format!("r{i:02}"),
                &format!("2026-10-05T00:00:{i:02}.000Z"),
                status,
            );
            let trimmed = store.save_workflow_run(&exec).unwrap();
            if i < 50 {
                assert!(trimmed.is_empty());
            } else {
                assert_eq!(trimmed, [format!("r{:02}", i - 48)]);
            }
        }
        let ids = store.list_workflow_run_ids().unwrap();
        assert_eq!(ids.len(), 50);
        assert!(ids.contains(&"r00".to_string()) && ids.contains(&"r01".to_string()));
        assert!(!ids.contains(&"r02".to_string()) && !ids.contains(&"r03".to_string()));
        assert_eq!(
            store.list_workflow_runs("wf", Some(3.0)).unwrap()[0]["runId"],
            "r51"
        );
        assert_eq!(store.list_running_runs().unwrap().len(), 2);
    }

    #[test]
    fn lists_runs_with_waiting_gates_by_kind() {
        let mut store = test_support::store();
        let mut a = run("wf", "a", "2026-10-05T00:00:01.000Z", "running");
        a.node_states = vec![node(
            json!({ "nodeId": "g", "status": "waiting", "waitingFor": "signIn" }),
        )];
        let mut b = run("wf", "b", "2026-10-05T00:00:02.000Z", "running");
        b.node_states = vec![
            node(json!({ "nodeId": "g1", "status": "waiting" })),
            node(json!({ "nodeId": "g2", "status": "waiting" })),
        ];
        let c = run("wf", "c", "2026-10-05T00:00:03.000Z", "running");
        for exec in [&a, &b, &c] {
            store.save_workflow_run(exec).unwrap();
        }
        let ids = |runs: Vec<Value>| {
            runs.into_iter()
                .map(|r| r["runId"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(store.list_runs_with_waiting_gates(None).unwrap()),
            [json!("b"), json!("a")]
        );
        assert_eq!(
            ids(store.list_runs_with_waiting_gates(Some("signIn")).unwrap()),
            [json!("a")]
        );
        assert_eq!(
            ids(store.list_runs_with_waiting_gates(Some("other")).unwrap()).len(),
            2
        );
    }

    #[test]
    fn lists_all_runs_by_workspace_without_orphans_and_by_task() {
        let mut store = test_support::store();
        add_workflow(&store, "w1", "personal");
        add_workflow(&store, "w2", "team");
        let mut r1 = run("w1", "r1", "2026-10-05T00:00:01.000Z", "completed");
        r1.trigger_task_id = Some("t1".into());
        let r2 = run("w2", "r2", "2026-10-05T00:00:02.000Z", "completed");
        let mut orphan = run("gone", "r3", "2026-10-05T00:00:03.000Z", "completed");
        orphan.node_states = vec![node(
            json!({ "nodeId": "n", "status": "completed", "taskId": "t1" }),
        )];
        for exec in [&r1, &r2, &orphan] {
            store.save_workflow_run(exec).unwrap();
        }
        let all = store.list_all_workflow_runs(None, None).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0]["runId"], "r3");
        assert!(all[0].get("workflowName").is_none());
        assert_eq!(all[1]["workflowName"], "Flow w2");
        assert_eq!(
            store
                .list_all_workflow_runs(Some(""), Some(0.0))
                .unwrap()
                .len(),
            1
        );

        let personal = store
            .list_all_workflow_runs(Some("personal"), None)
            .unwrap();
        assert_eq!(personal.len(), 1);
        assert_eq!(personal[0]["runId"], "r1");

        let by_task = store.list_workflow_runs_by_task("t1", None).unwrap();
        let ids: Vec<_> = by_task.iter().map(|r| r["runId"].clone()).collect();
        assert_eq!(ids, [json!("r3"), json!("r1")]);
        assert_eq!(by_task[1]["workflowName"], "Flow w1");
    }

    #[test]
    fn corrupt_json_columns_read_as_absent() {
        let mut store = test_support::store();
        store
            .save_workflow_run(&run("wf", "r", "t", "running"))
            .unwrap();
        store
            .conn()
            .execute(
                "UPDATE workflow_runs SET inputs = '{bad', definition = '[1]' WHERE id = 'r'",
                [],
            )
            .unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO workflow_run_nodes (run_id, node_id, status, structured_output, feedback) VALUES ('r', 'n', 'done', '\"x\"', 'nope')",
                [],
            )
            .unwrap();
        let got = store.get_workflow_run("r").unwrap().unwrap();
        assert!(got.get("inputs").is_none() && got.get("definition").is_none());
        assert_eq!(
            got["nodeStates"],
            json!([{ "nodeId": "n", "status": "done" }])
        );
    }
}

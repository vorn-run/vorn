//! Projects, workflows, identity, workspaces, session groups and SSH keys:
//! the targeted calls (`dbListProjects` and the rest). `loadConfig` and
//! `saveConfig` read the same rows through the row mappers here.

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params, params_from_iter, OptionalExtension, Row};
use serde_json::{Map, Value};
use vorn_protocol::{
    DeviceToken, DeviceTokenSecret, NewDeviceToken, ProjectConfig, SessionGroupConfig, SshKey,
    SshKeyMeta, User, UserRole, WorkflowDefinition, WorkspaceConfig,
};

use crate::sql::{
    get_f64, get_opt_f64, get_opt_text, get_text, json_if_truthy, json_text, num, opt_num,
    parse_json, truthy,
};
use crate::tasks::bind;
use crate::{Result, Store};

/// A `projects` row as `rowToProject` maps it: `preferredAgents` and
/// `hostIds` are parsed (and throw on bad JSON, as `JSON.parse` does), and a
/// row from before workspaces belongs to `personal`.
pub(crate) fn row_to_project(row: &Row<'_>) -> Result<ProjectConfig> {
    Ok(ProjectConfig {
        name: get_text(row, "name")?,
        path: get_text(row, "path")?,
        preferred_agents: parse_json(&get_text(row, "preferred_agents")?)?,
        icon: get_opt_text(row, "icon")?,
        icon_color: get_opt_text(row, "icon_color")?,
        host_ids: get_opt_text(row, "host_ids")?
            .map(|text| parse_json(&text))
            .transpose()?,
        workspace_id: Some(personal_unless_set(get_opt_text(row, "workspace_id")?)),
    })
}

/// A `workflows` row as `rowToWorkflow` maps it. `enabled` is `=== 1`, so
/// only the number one is on.
pub(crate) fn row_to_workflow(row: &Row<'_>) -> Result<WorkflowDefinition> {
    let enabled = match row.get_ref("enabled")? {
        ValueRef::Integer(i) => i == 1,
        ValueRef::Real(f) => f == 1.0,
        _ => false,
    };
    Ok(WorkflowDefinition {
        id: get_text(row, "id")?,
        name: get_text(row, "name")?,
        icon: get_text(row, "icon")?,
        icon_color: get_text(row, "icon_color")?,
        nodes: parse_json(&get_text(row, "nodes")?)?,
        edges: parse_json(&get_text(row, "edges")?)?,
        enabled,
        last_run_at: get_opt_text(row, "last_run_at")?,
        last_run_status: get_opt_text(row, "last_run_status")?,
        stagger_delay_ms: get_opt_f64(row, "stagger_delay_ms")?,
        workspace_id: Some(personal_unless_set(get_opt_text(row, "workspace_id")?)),
        auto_cleanup_worktrees: None,
    })
}

/// A `session_groups` row as `rowToSessionGroup` maps it.
pub(crate) fn row_to_session_group(row: &Row<'_>) -> Result<SessionGroupConfig> {
    Ok(SessionGroupConfig {
        id: get_text(row, "id")?,
        name: get_text(row, "name")?,
        icon: get_opt_text(row, "icon")?,
        icon_color: get_opt_text(row, "icon_color")?,
        order: get_f64(row, "order")?,
        workspace_id: personal_unless_set(get_opt_text(row, "workspace_id")?),
    })
}

/// A `workspaces` row as `rowToWorkspace` maps it.
pub(crate) fn row_to_workspace(row: &Row<'_>) -> Result<WorkspaceConfig> {
    Ok(WorkspaceConfig {
        id: get_text(row, "id")?,
        name: get_text(row, "name")?,
        icon: get_opt_text(row, "icon")?,
        icon_color: get_opt_text(row, "icon_color")?,
        order: get_f64(row, "order")?,
    })
}

/// `workspaceId ?? 'personal'`.
fn personal_unless_set(workspace_id: Option<String>) -> String {
    workspace_id.unwrap_or_else(|| "personal".to_owned())
}

/// Collects the `UPDATE ... SET` clauses a partial update names, in the
/// order the TypeScript tests its keys.
#[derive(Default)]
struct Sets {
    clauses: Vec<&'static str>,
    args: Vec<SqlValue>,
}

impl Sets {
    fn push(&mut self, clause: &'static str, value: SqlValue) {
        self.clauses.push(clause);
        self.args.push(value);
    }

    /// Runs `UPDATE <table> SET ... WHERE <key> = ?` and returns the rows
    /// changed, or 0 without a statement when nothing was named.
    fn run(mut self, store: &Store, table: &str, key: &str, id: &str) -> Result<usize> {
        if self.clauses.is_empty() {
            return Ok(0);
        }
        self.args.push(SqlValue::Text(id.to_owned()));
        let sql = format!(
            "UPDATE {table} SET {} WHERE {key} = ?",
            self.clauses.join(", ")
        );
        Ok(store.conn().execute(&sql, params_from_iter(self.args))?)
    }
}

/// Runs `sql` and maps every row it returns.
fn query_all<T>(
    store: &Store,
    sql: &str,
    params: impl rusqlite::Params,
    map: impl Fn(&Row<'_>) -> Result<T>,
) -> Result<Vec<T>> {
    let mut stmt = store.conn().prepare(sql)?;
    let mut rows = stmt.query(params)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map(row)?);
    }
    Ok(out)
}

/// Runs `sql` and maps its first row, if any.
fn query_one<T>(
    store: &Store,
    sql: &str,
    params: impl rusqlite::Params,
    map: impl Fn(&Row<'_>) -> Result<T>,
) -> Result<Option<T>> {
    let mut stmt = store.conn().prepare(sql)?;
    let mut rows = stmt.query(params)?;
    rows.next()?.map(map).transpose()
}

fn row_to_device_token(row: &Row<'_>) -> Result<DeviceToken> {
    Ok(DeviceToken {
        id: get_text(row, "id")?,
        user_id: get_text(row, "user_id")?,
        name: get_text(row, "name")?,
        created_at: get_text(row, "created_at")?,
        last_seen_at: get_opt_text(row, "last_seen_at")?,
        revoked_at: get_opt_text(row, "revoked_at")?,
    })
}

// Projects

impl Store {
    pub fn db_list_projects(&self) -> Result<Vec<ProjectConfig>> {
        query_all(self, "SELECT * FROM projects", [], row_to_project)
    }

    pub fn db_get_project(&self, name: &str) -> Result<Option<ProjectConfig>> {
        query_one(
            self,
            "SELECT * FROM projects WHERE name = ?",
            [name],
            row_to_project,
        )
    }

    pub fn db_insert_project(&self, project: &ProjectConfig) -> Result<()> {
        self.conn().execute(
            "INSERT INTO projects (name, path, preferred_agents, icon, icon_color, host_ids, workspace_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                project.name,
                project.path,
                json_text(&project.preferred_agents)?,
                project.icon,
                project.icon_color,
                json_if_truthy(project.host_ids.as_ref())?,
                project.workspace_id.as_deref().unwrap_or("personal"),
            ],
        )?;
        Ok(())
    }

    /// `updates` is the partial project as JSON; a key present is one the
    /// TypeScript sees as `!== undefined`. `hostIds` is stringified whatever
    /// it is, so `null` stores the text `null`, as it does there.
    pub fn db_update_project(&self, name: &str, updates: &Map<String, Value>) -> Result<()> {
        let mut sets = Sets::default();
        if let Some(v) = updates.get("path") {
            sets.push("path = ?", bind(v));
        }
        if let Some(v) = updates.get("preferredAgents") {
            sets.push("preferred_agents = ?", SqlValue::Text(json_text(v)?));
        }
        if let Some(v) = updates.get("icon") {
            sets.push("icon = ?", bind(v));
        }
        if let Some(v) = updates.get("iconColor") {
            sets.push("icon_color = ?", bind(v));
        }
        if let Some(v) = updates.get("hostIds") {
            sets.push("host_ids = ?", SqlValue::Text(json_text(v)?));
        }
        if let Some(v) = updates.get("workspaceId") {
            sets.push("workspace_id = ?", bind(v));
        }
        sets.run(self, "projects", "name", name)?;
        Ok(())
    }

    /// Removes the project and its tasks together.
    pub fn db_delete_project(&mut self, name: &str) -> Result<()> {
        let tx = self.conn_mut().transaction()?;
        tx.execute("DELETE FROM tasks WHERE project_name = ?", [name])?;
        tx.execute("DELETE FROM projects WHERE name = ?", [name])?;
        tx.commit()?;
        Ok(())
    }
}

// Workflows

impl Store {
    pub fn db_list_workflows(&self) -> Result<Vec<WorkflowDefinition>> {
        query_all(self, "SELECT * FROM workflows", [], row_to_workflow)
    }

    pub fn db_get_workflow(&self, id: &str) -> Result<Option<WorkflowDefinition>> {
        query_one(
            self,
            "SELECT * FROM workflows WHERE id = ?",
            [id],
            row_to_workflow,
        )
    }

    pub fn db_insert_workflow(&self, workflow: &WorkflowDefinition) -> Result<()> {
        self.conn().execute(
            "INSERT INTO workflows (id, name, icon, icon_color, nodes, edges, enabled, last_run_at, last_run_status, stagger_delay_ms, workspace_id)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                workflow.id,
                workflow.name,
                workflow.icon,
                workflow.icon_color,
                json_text(&workflow.nodes)?,
                json_text(&workflow.edges)?,
                i64::from(workflow.enabled),
                workflow.last_run_at,
                workflow.last_run_status,
                opt_num(workflow.stagger_delay_ms),
                workflow.workspace_id.as_deref().unwrap_or("personal"),
            ],
        )?;
        Ok(())
    }

    /// Returns the rows changed: 0 when no row has `id`, and also when
    /// `updates` names no column this writes (then no statement runs). It
    /// answers existence only for a caller that names a column, as
    /// `workflow:setEnabled` always does.
    pub fn db_update_workflow(&self, id: &str, updates: &Map<String, Value>) -> Result<i64> {
        let mut sets = Sets::default();
        if let Some(v) = updates.get("name") {
            sets.push("name = ?", bind(v));
        }
        if let Some(v) = updates.get("nodes") {
            sets.push("nodes = ?", SqlValue::Text(json_text(v)?));
        }
        if let Some(v) = updates.get("edges") {
            sets.push("edges = ?", SqlValue::Text(json_text(v)?));
        }
        if let Some(v) = updates.get("icon") {
            sets.push("icon = ?", bind(v));
        }
        if let Some(v) = updates.get("iconColor") {
            sets.push("icon_color = ?", bind(v));
        }
        if let Some(v) = updates.get("enabled") {
            sets.push("enabled = ?", SqlValue::Integer(i64::from(truthy(v))));
        }
        if let Some(v) = updates.get("staggerDelayMs") {
            sets.push("stagger_delay_ms = ?", bind(v));
        }
        if let Some(v) = updates.get("workspaceId") {
            sets.push("workspace_id = ?", bind(v));
        }
        let changed = sets.run(self, "workflows", "id", id)?;
        Ok(i64::try_from(changed).unwrap_or(i64::MAX))
    }

    pub fn db_delete_workflow(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM workflows WHERE id = ?", [id])?;
        Ok(())
    }

    /// Records a run's outcome without a full config save.
    pub fn update_workflow_run_status(
        &self,
        id: &str,
        last_run_at: &str,
        last_run_status: &str,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE workflows SET last_run_at = ?, last_run_status = ? WHERE id = ?",
            params![last_run_at, last_run_status, id],
        )?;
        Ok(())
    }
}

// Identity and device tokens

impl Store {
    /// The seeded owner, present on any database past migration 14.
    pub fn db_get_owner_user(&self) -> Result<Option<User>> {
        query_one(
            self,
            "SELECT * FROM users WHERE role = 'owner' ORDER BY created_at LIMIT 1",
            [],
            |row| {
                Ok(User {
                    id: get_text(row, "id")?,
                    name: get_text(row, "name")?,
                    role: UserRole(get_text(row, "role")?),
                    created_at: get_text(row, "created_at")?,
                })
            },
        )
    }

    pub fn db_insert_device_token(&self, token: &NewDeviceToken) -> Result<()> {
        self.conn().execute(
            "INSERT INTO device_tokens (id, user_id, name, token_hash, created_at)
       VALUES (?, ?, ?, ?, ?)",
            params![
                token.id,
                token.user_id,
                token.name,
                token.token_hash,
                token.created_at
            ],
        )?;
        Ok(())
    }

    /// Carries the hash, for verification only; [`DeviceToken`] never does.
    pub fn db_get_device_token_secret(&self, id: &str) -> Result<Option<DeviceTokenSecret>> {
        query_one(
            self,
            "SELECT id, user_id, token_hash, revoked_at FROM device_tokens WHERE id = ?",
            [id],
            |row| {
                Ok(DeviceTokenSecret {
                    id: get_text(row, "id")?,
                    user_id: get_text(row, "user_id")?,
                    token_hash: get_text(row, "token_hash")?,
                    revoked_at: get_opt_text(row, "revoked_at")?,
                })
            },
        )
    }

    /// Columns are named rather than `*`, so the hash never leaves the store.
    pub fn db_list_device_tokens(&self) -> Result<Vec<DeviceToken>> {
        query_all(
            self,
            "SELECT id, user_id, name, created_at, last_seen_at, revoked_at
       FROM device_tokens ORDER BY created_at",
            [],
            row_to_device_token,
        )
    }

    pub fn db_has_device_tokens(&self) -> Result<bool> {
        let found: Option<i64> = self
            .conn()
            .query_row("SELECT 1 FROM device_tokens LIMIT 1", [], |row| row.get(0))
            .optional()?;
        Ok(found.is_some())
    }

    /// False when the id is unknown or the token was already revoked.
    pub fn db_revoke_device_token(&self, id: &str, revoked_at: &str) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE device_tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL",
            params![revoked_at, id],
        )?;
        Ok(changed > 0)
    }

    pub fn db_touch_device_token(&self, id: &str, seen_at: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE device_tokens SET last_seen_at = ? WHERE id = ?",
            params![seen_at, id],
        )?;
        Ok(())
    }
}

// Workspaces

impl Store {
    pub fn db_list_workspaces(&self) -> Result<Vec<WorkspaceConfig>> {
        query_all(
            self,
            r#"SELECT * FROM workspaces ORDER BY "order""#,
            [],
            row_to_workspace,
        )
    }

    pub fn db_insert_workspace(&self, workspace: &WorkspaceConfig) -> Result<()> {
        self.conn().execute(
            r#"INSERT INTO workspaces (id, name, icon, icon_color, "order") VALUES (?, ?, ?, ?, ?)"#,
            params![
                workspace.id,
                workspace.name,
                workspace.icon,
                workspace.icon_color,
                num(workspace.order),
            ],
        )?;
        Ok(())
    }

    pub fn db_update_workspace(&self, id: &str, updates: &Map<String, Value>) -> Result<()> {
        let mut sets = Sets::default();
        if let Some(v) = updates.get("name") {
            sets.push("name = ?", bind(v));
        }
        if let Some(v) = updates.get("icon") {
            sets.push("icon = ?", bind(v));
        }
        if let Some(v) = updates.get("iconColor") {
            sets.push("icon_color = ?", bind(v));
        }
        if let Some(v) = updates.get("order") {
            sets.push(r#""order" = ?"#, bind(v));
        }
        sets.run(self, "workspaces", "id", id)?;
        Ok(())
    }

    /// Moves the workspace's projects and workflows to `personal`; its
    /// session groups die with it, and their sessions come back ungrouped.
    pub fn db_delete_workspace(&mut self, id: &str) -> Result<()> {
        let tx = self.conn_mut().transaction()?;
        tx.execute(
            "UPDATE sessions SET group_id = NULL WHERE group_id IN (SELECT id FROM session_groups WHERE workspace_id = ?)",
            [id],
        )?;
        tx.execute("DELETE FROM session_groups WHERE workspace_id = ?", [id])?;
        tx.execute(
            "UPDATE projects SET workspace_id = 'personal' WHERE workspace_id = ?",
            [id],
        )?;
        tx.execute(
            "UPDATE workflows SET workspace_id = 'personal' WHERE workspace_id = ?",
            [id],
        )?;
        tx.execute("DELETE FROM workspaces WHERE id = ?", [id])?;
        tx.commit()?;
        Ok(())
    }
}

// Session groups

impl Store {
    pub fn db_list_session_groups(&self) -> Result<Vec<SessionGroupConfig>> {
        query_all(
            self,
            r#"SELECT * FROM session_groups ORDER BY "order""#,
            [],
            row_to_session_group,
        )
    }

    pub fn db_insert_session_group(&self, group: &SessionGroupConfig) -> Result<()> {
        self.conn().execute(
            r#"INSERT INTO session_groups (id, name, icon, icon_color, "order", workspace_id) VALUES (?, ?, ?, ?, ?, ?)"#,
            params![
                group.id,
                group.name,
                group.icon,
                group.icon_color,
                num(group.order),
                group.workspace_id,
            ],
        )?;
        Ok(())
    }

    pub fn db_update_session_group(&self, id: &str, updates: &Map<String, Value>) -> Result<()> {
        let mut sets = Sets::default();
        if let Some(v) = updates.get("name") {
            sets.push("name = ?", bind(v));
        }
        if let Some(v) = updates.get("icon") {
            sets.push("icon = ?", bind(v));
        }
        if let Some(v) = updates.get("iconColor") {
            sets.push("icon_color = ?", bind(v));
        }
        if let Some(v) = updates.get("order") {
            sets.push(r#""order" = ?"#, bind(v));
        }
        if let Some(v) = updates.get("workspaceId") {
            sets.push("workspace_id = ?", bind(v));
        }
        sets.run(self, "session_groups", "id", id)?;
        Ok(())
    }

    /// Ungroups the group's sessions first: deleting a group never kills one.
    pub fn db_delete_session_group(&mut self, id: &str) -> Result<()> {
        let tx = self.conn_mut().transaction()?;
        tx.execute(
            "UPDATE sessions SET group_id = NULL WHERE group_id = ?",
            [id],
        )?;
        tx.execute("DELETE FROM session_groups WHERE id = ?", [id])?;
        tx.commit()?;
        Ok(())
    }
}

// SSH keys

impl Store {
    pub fn db_save_ssh_key(&self, key: &SshKey) -> Result<()> {
        self.conn().execute(
            "INSERT INTO ssh_keys (id, label, encrypted_private_key, public_key, certificate, key_type, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                key.id,
                key.label,
                key.encrypted_private_key,
                key.public_key,
                key.certificate,
                key.key_type,
                key.created_at,
            ],
        )?;
        Ok(())
    }

    /// Every key without its private half or certificate.
    pub fn db_list_ssh_keys(&self) -> Result<Vec<SshKeyMeta>> {
        query_all(
            self,
            "SELECT id, label, key_type, public_key, created_at FROM ssh_keys",
            [],
            |row| {
                Ok(SshKeyMeta {
                    id: get_text(row, "id")?,
                    label: get_text(row, "label")?,
                    key_type: get_opt_text(row, "key_type")?,
                    public_key: get_opt_text(row, "public_key")?,
                    created_at: get_text(row, "created_at")?,
                })
            },
        )
    }

    pub fn db_get_ssh_key(&self, id: &str) -> Result<Option<SshKey>> {
        query_one(self, "SELECT * FROM ssh_keys WHERE id = ?", [id], |row| {
            Ok(SshKey {
                id: get_text(row, "id")?,
                label: get_text(row, "label")?,
                encrypted_private_key: get_text(row, "encrypted_private_key")?,
                public_key: get_opt_text(row, "public_key")?,
                certificate: get_opt_text(row, "certificate")?,
                key_type: get_opt_text(row, "key_type")?,
                created_at: get_text(row, "created_at")?,
            })
        })
    }

    pub fn db_delete_ssh_key(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM ssh_keys WHERE id = ?", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support;
    use serde_json::json;

    fn project(name: &str) -> vorn_protocol::ProjectConfig {
        serde_json::from_value(json!({
            "name": name,
            "path": "/p",
            "preferredAgents": ["claude"]
        }))
        .unwrap()
    }

    fn workflow(id: &str) -> vorn_protocol::WorkflowDefinition {
        serde_json::from_value(json!({
            "id": id,
            "name": "W",
            "icon": "Zap",
            "iconColor": "#fff",
            "nodes": [{ "id": "n1" }],
            "edges": [],
            "enabled": true
        }))
        .unwrap()
    }

    #[test]
    fn projects_round_trip_and_update() {
        let mut store = test_support::store();
        store.db_insert_project(&project("a")).unwrap();
        let json = serde_json::to_value(store.db_get_project("a").unwrap().unwrap()).unwrap();
        assert_eq!(
            json,
            json!({ "name": "a", "path": "/p", "preferredAgents": ["claude"], "workspaceId": "personal" })
        );

        let updates = json!({ "hostIds": ["h1"], "icon": "Folder", "path": "/q" });
        store
            .db_update_project("a", updates.as_object().unwrap())
            .unwrap();
        let p = store.db_get_project("a").unwrap().unwrap();
        assert_eq!(p.host_ids, Some(json!(["h1"])));
        assert_eq!(p.icon.as_deref(), Some("Folder"));
        assert_eq!(p.path, "/q");
        assert_eq!(store.db_list_projects().unwrap().len(), 1);

        store.db_delete_project("a").unwrap();
        assert!(store.db_get_project("a").unwrap().is_none());
    }

    #[test]
    fn deleting_a_project_takes_its_tasks() {
        let mut store = test_support::store();
        store.db_insert_project(&project("a")).unwrap();
        let task = serde_json::from_value(json!({
            "id": "t", "projectName": "a", "title": "T", "description": "",
            "status": "todo", "order": 0,
            "createdAt": "x", "updatedAt": "x"
        }))
        .unwrap();
        store.db_insert_task(&task).unwrap();
        store.db_delete_project("a").unwrap();
        assert!(store.db_get_task("t").unwrap().is_none());
    }

    #[test]
    fn a_bad_json_column_is_an_error() {
        let store = test_support::store();
        store
            .conn()
            .execute(
                "INSERT INTO projects (name, path, preferred_agents) VALUES ('x', '/', 'nope')",
                [],
            )
            .unwrap();
        assert!(matches!(
            store.db_get_project("x"),
            Err(crate::Error::Json(_))
        ));
    }

    #[test]
    fn workflows_round_trip_and_count_changes() {
        let store = test_support::store();
        store.db_insert_workflow(&workflow("w")).unwrap();
        let w = store.db_get_workflow("w").unwrap().unwrap();
        assert!(w.enabled);
        assert_eq!(w.nodes, json!([{ "id": "n1" }]));
        assert_eq!(w.workspace_id.as_deref(), Some("personal"));
        let json = serde_json::to_value(&w).unwrap();
        assert!(json.get("lastRunAt").is_none(), "{json}");

        let none = serde_json::Map::new();
        assert_eq!(store.db_update_workflow("w", &none).unwrap(), 0);
        let off = json!({ "enabled": false, "unknown": 1 });
        assert_eq!(
            store
                .db_update_workflow("w", off.as_object().unwrap())
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .db_update_workflow("missing", off.as_object().unwrap())
                .unwrap(),
            0
        );
        assert!(!store.db_get_workflow("w").unwrap().unwrap().enabled);

        store
            .update_workflow_run_status("w", "2026-10-05T00:00:00.000Z", "success")
            .unwrap();
        let w = store.db_get_workflow("w").unwrap().unwrap();
        assert_eq!(w.last_run_status.as_deref(), Some("success"));
        assert_eq!(store.db_list_workflows().unwrap().len(), 1);

        store.db_delete_workflow("w").unwrap();
        assert!(store.db_get_workflow("w").unwrap().is_none());
    }

    #[test]
    fn device_tokens_revoke_once_and_hide_the_hash() {
        let store = test_support::store();
        let owner = store.db_get_owner_user().unwrap().expect("seeded owner");
        assert_eq!(owner.role.0, "owner");
        assert!(!store.db_has_device_tokens().unwrap());

        let token = vorn_protocol::NewDeviceToken {
            id: "d".into(),
            user_id: owner.id.clone(),
            name: "phone".into(),
            token_hash: "hash".into(),
            created_at: "2026-10-05T00:00:00.000Z".into(),
        };
        store.db_insert_device_token(&token).unwrap();
        assert!(store.db_has_device_tokens().unwrap());

        let listed = serde_json::to_value(store.db_list_device_tokens().unwrap()).unwrap();
        assert_eq!(listed[0]["lastSeenAt"], json!(null));
        assert!(listed[0].get("tokenHash").is_none());

        store.db_touch_device_token("d", "later").unwrap();
        assert_eq!(
            store.db_list_device_tokens().unwrap()[0]
                .last_seen_at
                .as_deref(),
            Some("later")
        );

        assert!(store.db_revoke_device_token("d", "now").unwrap());
        assert!(!store.db_revoke_device_token("d", "again").unwrap());
        assert!(!store.db_revoke_device_token("missing", "now").unwrap());
        let secret = store.db_get_device_token_secret("d").unwrap().unwrap();
        assert_eq!(secret.token_hash, "hash");
        assert_eq!(secret.revoked_at.as_deref(), Some("now"));
        assert!(store.db_get_device_token_secret("x").unwrap().is_none());
    }

    #[test]
    fn deleting_a_workspace_rehomes_and_ungroups() {
        let mut store = test_support::store();
        let ws = serde_json::from_value(json!({ "id": "w2", "name": "Work", "order": 1 })).unwrap();
        store.db_insert_workspace(&ws).unwrap();
        let updates = json!({ "name": "Job", "iconColor": "#000" });
        store
            .db_update_workspace("w2", updates.as_object().unwrap())
            .unwrap();
        let listed = store.db_list_workspaces().unwrap();
        let w2 = listed.iter().find(|w| w.id == "w2").unwrap();
        assert_eq!(w2.name, "Job");
        assert_eq!(w2.icon, None);

        let mut p = project("p");
        p.workspace_id = Some("w2".into());
        store.db_insert_project(&p).unwrap();
        let mut wf = workflow("f");
        wf.workspace_id = Some("w2".into());
        store.db_insert_workflow(&wf).unwrap();
        let group = serde_json::from_value(
            json!({ "id": "g", "name": "G", "order": 0, "workspaceId": "w2" }),
        )
        .unwrap();
        store.db_insert_session_group(&group).unwrap();

        store.db_delete_workspace("w2").unwrap();
        assert!(store
            .db_list_workspaces()
            .unwrap()
            .iter()
            .all(|w| w.id != "w2"));
        assert!(store.db_list_session_groups().unwrap().is_empty());
        assert_eq!(
            store
                .db_get_project("p")
                .unwrap()
                .unwrap()
                .workspace_id
                .as_deref(),
            Some("personal")
        );
        assert_eq!(
            store
                .db_get_workflow("f")
                .unwrap()
                .unwrap()
                .workspace_id
                .as_deref(),
            Some("personal")
        );
    }

    #[test]
    fn session_groups_update_and_delete() {
        let mut store = test_support::store();
        for (id, order) in [("b", 2), ("a", 1)] {
            let g = serde_json::from_value(
                json!({ "id": id, "name": id, "order": order, "workspaceId": "personal" }),
            )
            .unwrap();
            store.db_insert_session_group(&g).unwrap();
        }
        let ids: Vec<_> = store
            .db_list_session_groups()
            .unwrap()
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(ids, ["a", "b"]);
        let updates = json!({ "order": 0.5, "icon": "Star" });
        store
            .db_update_session_group("b", updates.as_object().unwrap())
            .unwrap();
        let first = &store.db_list_session_groups().unwrap()[0];
        assert_eq!((first.id.as_str(), first.order), ("b", 0.5));
        assert_eq!(first.icon.as_deref(), Some("Star"));

        store.db_delete_session_group("b").unwrap();
        assert_eq!(store.db_list_session_groups().unwrap().len(), 1);
    }

    #[test]
    fn ssh_keys_list_without_secrets() {
        let store = test_support::store();
        let key = vorn_protocol::SshKey {
            id: "k".into(),
            label: "laptop".into(),
            encrypted_private_key: "enc".into(),
            public_key: None,
            certificate: Some("cert".into()),
            key_type: Some("ed25519".into()),
            created_at: "x".into(),
        };
        store.db_save_ssh_key(&key).unwrap();
        let listed = serde_json::to_value(store.db_list_ssh_keys().unwrap()).unwrap();
        assert_eq!(
            listed,
            json!([{ "id": "k", "label": "laptop", "keyType": "ed25519", "createdAt": "x" }])
        );
        let got = store.db_get_ssh_key("k").unwrap().unwrap();
        assert_eq!(got.certificate.as_deref(), Some("cert"));
        assert_eq!(got.public_key, None);
        store.db_delete_ssh_key("k").unwrap();
        assert!(store.db_get_ssh_key("k").unwrap().is_none());
    }
}

//! The whole-config snapshot: `loadConfig` and `saveConfig`.
//!
//! A save rewrites the collections it carries from the snapshot, but a row
//! the snapshot leaves out is deleted only if the saving client could have
//! seen it: every revisioned row records the save that wrote it, and a row
//! written after the client's base revision survives. That is what keeps two
//! clients open at once from deleting each other's work.

use std::collections::HashSet;

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, Connection, Row, Transaction};
use serde::Deserialize;
use serde_json::{Map, Number, Value};
use vorn_protocol::{
    AgentCommandConfig, AuthMethod, ProjectConfig, RemoteHost, SessionGroupConfig, TaskConfig,
    WorkflowDefinition, WorkspaceConfig,
};

use crate::catalog::{row_to_project, row_to_session_group, row_to_workflow, row_to_workspace};
use crate::schema::CONFIG_REVISION_KEY;
use crate::sql::{
    format_js_number, get_f64, get_opt_text, get_text, json_if_truthy, json_text, num, opt_num,
    parse_json,
};
use crate::tasks::row_to_task;
use crate::{Result, Store};

/// `Number.MAX_SAFE_INTEGER`: the base revision of a caller that does not
/// track revisions (the CLI, a test, the server saving its own port), so its
/// snapshot prunes everything it omits.
const NO_BASE_REVISION: f64 = 9_007_199_254_740_991.0;

/// The parts of an `AppConfig` a save reads. A collection that is absent or
/// `null` is `?? []` there and `None` here; `defaults` and `projects` are
/// required, as `Object.entries` and `.map` would throw without them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedConfig {
    revision: Option<f64>,
    defaults: Map<String, Value>,
    projects: Vec<ProjectConfig>,
    workflows: Option<Vec<WorkflowDefinition>>,
    /// Values stay JSON: an entry whose value is falsy is kept from pruning
    /// but not written, as `if (cmd)` skips it.
    agent_commands: Option<Map<String, Value>>,
    remote_hosts: Option<Vec<RemoteHost>>,
    tasks: Option<Vec<TaskConfig>>,
    workspaces: Option<Vec<WorkspaceConfig>>,
    session_groups: Option<Vec<SessionGroupConfig>>,
}

impl Store {
    /// The `AppConfig` object, keys in the order `loadConfig` writes them.
    pub fn load_config(&self) -> Result<Value> {
        let conn = self.conn();
        let defaults = self.load_defaults()?;
        let projects = collect(conn, "SELECT * FROM projects", row_to_project)?;
        let mut agent_commands = load_agent_commands(conn)?;
        let workflows = collect(conn, "SELECT * FROM workflows", row_to_workflow)?;
        let remote_hosts = collect(conn, "SELECT * FROM remote_hosts", row_to_remote_host)?;
        let tasks = collect(conn, r#"SELECT * FROM tasks ORDER BY "order""#, row_to_task)?;
        let workspaces = collect(
            conn,
            r#"SELECT * FROM workspaces ORDER BY "order""#,
            row_to_workspace,
        )?;
        let session_groups = collect(
            conn,
            r#"SELECT * FROM session_groups ORDER BY "order""#,
            row_to_session_group,
        )?;
        if agent_commands.is_empty() {
            agent_commands = self.options()?.default_agent_commands.clone();
        }

        let mut config = Map::new();
        config.insert("version".into(), Value::from(1));
        config.insert(
            "revision".into(),
            js_number_value(read_config_revision(conn)?),
        );
        config.insert("defaults".into(), Value::Object(defaults));
        config.insert("projects".into(), serde_json::to_value(projects)?);
        config.insert("agentCommands".into(), Value::Object(agent_commands));
        config.insert("workflows".into(), serde_json::to_value(workflows)?);
        config.insert("remoteHosts".into(), serde_json::to_value(remote_hosts)?);
        config.insert("tasks".into(), serde_json::to_value(tasks)?);
        config.insert("workspaces".into(), serde_json::to_value(workspaces)?);
        config.insert(
            "sessionGroups".into(),
            serde_json::to_value(session_groups)?,
        );
        Ok(Value::Object(config))
    }

    /// `defaults` as `loadDefaults` builds it: an explicit list of keys, so a
    /// row for a key not listed here does not load. `??` keys fall back when
    /// the row is missing or holds `null`; the conditional keys appear when
    /// the row exists, even holding `null`.
    fn load_defaults(&self) -> Result<Map<String, Value>> {
        let stored = {
            let mut stmt = self.conn().prepare("SELECT key, value FROM defaults")?;
            let mut rows = stmt.query([])?;
            let mut stored = Map::new();
            while let Some(row) = rows.next()? {
                let value = parse_json(&get_text(row, "value")?)?;
                stored.insert(get_text(row, "key")?, value);
            }
            stored
        };
        let or = |key: &str, fallback: Value| -> Value {
            stored
                .get(key)
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(fallback)
        };

        let mut out = Map::new();
        let set_if_present = |out: &mut Map<String, Value>, keys: &[&str]| {
            for key in keys {
                if let Some(value) = stored.get(*key) {
                    out.insert((*key).to_owned(), value.clone());
                }
            }
        };
        out.insert(
            "shell".into(),
            or(
                "shell",
                Value::String(self.options()?.default_shell.clone()),
            ),
        );
        out.insert("fontSize".into(), or("fontSize", Value::from(13)));
        out.insert("theme".into(), or("theme", Value::from("dark")));
        set_if_present(
            &mut out,
            &[
                "rowHeight",
                "defaultAgent",
                "notifications",
                "hasSeenOnboarding",
            ],
        );
        out.insert(
            "reopenSessions".into(),
            or("reopenSessions", Value::Bool(true)),
        );
        out.insert(
            "startAtLogin".into(),
            or("startAtLogin", Value::Bool(false)),
        );
        // Array-checked rather than trusted: the row is user-editable JSON.
        if let Some(Value::Array(keys)) = stored.get("envPassthrough") {
            let strings = keys.iter().filter(|k| k.is_string()).cloned().collect();
            out.insert("envPassthrough".into(), Value::Array(strings));
        }
        out.insert(
            "domBlockRendering".into(),
            or("domBlockRendering", Value::Bool(true)),
        );
        out.insert(
            "minimalShellPrompt".into(),
            or("minimalShellPrompt", Value::Bool(true)),
        );
        out.insert(
            "keepSessionsRunning".into(),
            or("keepSessionsRunning", Value::Bool(true)),
        );
        set_if_present(
            &mut out,
            &[
                "widgetEnabled",
                "taskViewMode",
                "activeWorkspace",
                "mainViewMode",
                "layoutMode",
                "minimizedPlacement",
                "updateChannel",
                "webAccessEnabled",
                "mobileAccessEnabled",
                "serverPort",
                "networkAccessEnabled",
                "showHeadlessAgents",
                "headlessRetentionMinutes",
                "hasSeededDevServerWorkflow",
                "hasSeededDefaultTaskWorkflow",
                "updateAutoDownload",
                "headlessStepTimeoutMinutes",
                "enableHoverPreview",
                "worktreeRetention",
            ],
        );
        // Kept to booleans: an edited-in value must not read as a switch
        // that is on.
        if let Some(Value::Object(raw)) = stored.get("experimental") {
            let flags = raw
                .iter()
                .filter(|(_, v)| v.is_boolean())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            out.insert("experimental".into(), Value::Object(js_key_order(flags)));
        }
        Ok(out)
    }

    /// Writes a whole `AppConfig` snapshot in one transaction, as `saveConfig`
    /// does. `deleted_defaults` names the `defaults` keys the caller set to
    /// `undefined`, which JSON cannot carry: those rows are deleted, while a
    /// key simply absent from `defaults` is left as it is. A key is never in
    /// both, so deleting after the upserts loses nothing.
    pub fn save_config(&mut self, config: &Value, deleted_defaults: &[String]) -> Result<()> {
        let config = SavedConfig::deserialize(config)?;
        let default_workspace = self.options()?.default_workspace.clone();
        let tx = self.conn_mut().transaction()?;

        let base = config.revision.unwrap_or(NO_BASE_REVISION);
        let revision = read_config_revision(&tx)? + 1.0;
        let rev = num(revision);

        for (key, value) in &config.defaults {
            tx.execute(
                "INSERT INTO defaults (key, value) VALUES (?, ?)
       ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, json_text(value)?],
            )?;
        }
        for key in deleted_defaults {
            tx.execute("DELETE FROM defaults WHERE key = ?", [key])?;
        }

        // Projects: diff and upsert, so tasks referencing a project by name
        // never see it deleted and reinserted.
        let names: Vec<&str> = config.projects.iter().map(|p| p.name.as_str()).collect();
        prune_missing(&tx, "projects", "name", &names, base)?;
        for p in &config.projects {
            tx.execute(
                "INSERT INTO projects (name, path, preferred_agents, icon, icon_color, host_ids, workspace_id, row_revision)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(name) DO UPDATE SET
         row_revision = excluded.row_revision,
         path = excluded.path,
         preferred_agents = excluded.preferred_agents,
         icon = excluded.icon,
         icon_color = excluded.icon_color,
         host_ids = excluded.host_ids,
         workspace_id = excluded.workspace_id",
                params![
                    p.name,
                    p.path,
                    json_text(&p.preferred_agents)?,
                    p.icon,
                    p.icon_color,
                    json_if_truthy(p.host_ids.as_ref())?,
                    p.workspace_id.as_deref().unwrap_or("personal"),
                    rev,
                ],
            )?;
        }

        // Workflows: deleted when absent, with no revision check, but never
        // rewritten wholesale, so connector inbox rows and poll cursors that
        // cascade off them survive an ordinary save.
        let workflows = config.workflows.unwrap_or_default();
        let ids: HashSet<&str> = workflows.iter().map(|w| w.id.as_str()).collect();
        for existing in keys(&tx, "SELECT id FROM workflows")? {
            if !matches!(&existing, SqlValue::Text(id) if ids.contains(id.as_str())) {
                tx.execute("DELETE FROM workflows WHERE id = ?", [existing])?;
            }
        }
        for w in &workflows {
            tx.execute(
                "INSERT INTO workflows (id, name, icon, icon_color, nodes, edges, enabled, last_run_at, last_run_status, stagger_delay_ms, workspace_id)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(id) DO UPDATE SET
         name = excluded.name,
         icon = excluded.icon,
         icon_color = excluded.icon_color,
         nodes = excluded.nodes,
         edges = excluded.edges,
         enabled = excluded.enabled,
         last_run_at = excluded.last_run_at,
         last_run_status = excluded.last_run_status,
         stagger_delay_ms = excluded.stagger_delay_ms,
         workspace_id = excluded.workspace_id",
                params![
                    w.id,
                    w.name,
                    w.icon,
                    w.icon_color,
                    json_text(&w.nodes)?,
                    json_text(&w.edges)?,
                    i64::from(w.enabled),
                    w.last_run_at,
                    w.last_run_status,
                    opt_num(w.stagger_delay_ms),
                    w.workspace_id.as_deref().unwrap_or("personal"),
                ],
            )?;
        }

        let agent_commands = config.agent_commands.unwrap_or_default();
        let agent_types: Vec<&str> = agent_commands.keys().map(String::as_str).collect();
        prune_missing(&tx, "agent_commands", "agent_type", &agent_types, base)?;
        for (agent_type, cmd) in &agent_commands {
            if !crate::sql::truthy(cmd) {
                continue;
            }
            let cmd = AgentCommandConfig::deserialize(cmd)?;
            tx.execute(
                "INSERT INTO agent_commands (agent_type, command, args, headless_args, fallback_command, fallback_args, row_revision)
       VALUES (?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(agent_type) DO UPDATE SET
         row_revision = excluded.row_revision,
         command = excluded.command,
         args = excluded.args,
         headless_args = excluded.headless_args,
         fallback_command = excluded.fallback_command,
         fallback_args = excluded.fallback_args",
                params![
                    agent_type,
                    cmd.command,
                    json_text(&cmd.args)?,
                    json_if_truthy(cmd.headless_args.as_ref())?,
                    cmd.fallback_command,
                    json_if_truthy(cmd.fallback_args.as_ref())?,
                    rev,
                ],
            )?;
        }

        let remote_hosts = config.remote_hosts.unwrap_or_default();
        let host_ids: Vec<&str> = remote_hosts.iter().map(|h| h.id.as_str()).collect();
        prune_missing(&tx, "remote_hosts", "id", &host_ids, base)?;
        for h in &remote_hosts {
            tx.execute(
                "INSERT INTO remote_hosts (id, label, hostname, user, port, auth_method, ssh_key_path, credential_id, encrypted_password, ssh_options, row_revision)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(id) DO UPDATE SET
         row_revision = excluded.row_revision,
         label = excluded.label,
         hostname = excluded.hostname,
         user = excluded.user,
         port = excluded.port,
         auth_method = excluded.auth_method,
         ssh_key_path = excluded.ssh_key_path,
         credential_id = excluded.credential_id,
         encrypted_password = excluded.encrypted_password,
         ssh_options = excluded.ssh_options",
                params![
                    h.id,
                    h.label,
                    h.hostname,
                    h.user,
                    num(h.port),
                    h.auth_method.as_ref().map_or("agent", |a| a.0.as_str()),
                    h.ssh_key_path,
                    h.credential_id,
                    h.encrypted_password,
                    h.ssh_options,
                    rev,
                ],
            )?;
        }

        // Tasks: task_source_links cascade off them, so a wipe would orphan
        // the link to the issue a task came from.
        let tasks = config.tasks.unwrap_or_default();
        let task_ids: Vec<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
        prune_missing(&tx, "tasks", "id", &task_ids, base)?;
        for t in &tasks {
            tx.execute(
                r#"INSERT INTO tasks (id, project_name, title, description, status, "order", assigned_session_id, assigned_agent, agent_session_id, branch, use_worktree, created_at, updated_at, completed_at, archived_at, source_connector_id, source_external_url, source_external_id, row_revision)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(id) DO UPDATE SET
         row_revision = excluded.row_revision,
         project_name = excluded.project_name,
         title = excluded.title,
         description = excluded.description,
         status = excluded.status,
         "order" = excluded."order",
         assigned_session_id = excluded.assigned_session_id,
         assigned_agent = excluded.assigned_agent,
         agent_session_id = excluded.agent_session_id,
         branch = excluded.branch,
         use_worktree = excluded.use_worktree,
         created_at = excluded.created_at,
         updated_at = excluded.updated_at,
         completed_at = excluded.completed_at,
         archived_at = excluded.archived_at,
         source_connector_id = excluded.source_connector_id,
         source_external_url = excluded.source_external_url,
         source_external_id = excluded.source_external_id"#,
                params![
                    t.id,
                    t.project_name,
                    t.title,
                    t.description,
                    t.status.0,
                    num(t.order),
                    t.assigned_session_id,
                    t.assigned_agent.as_ref().map(|a| a.0.as_str()),
                    t.agent_session_id,
                    t.branch,
                    i64::from(t.use_worktree == Some(true)),
                    t.created_at,
                    t.updated_at,
                    t.completed_at,
                    t.archived_at,
                    t.source_connector_id,
                    t.source_external_url,
                    t.source_external_id,
                    rev,
                ],
            )?;
        }

        let workspaces = config.workspaces.unwrap_or_else(|| vec![default_workspace]);
        let workspace_ids: Vec<&str> = workspaces.iter().map(|w| w.id.as_str()).collect();
        prune_missing(&tx, "workspaces", "id", &workspace_ids, base)?;
        for ws in &workspaces {
            tx.execute(
                r#"INSERT INTO workspaces (id, name, icon, icon_color, "order", row_revision) VALUES (?, ?, ?, ?, ?, ?)
       ON CONFLICT(id) DO UPDATE SET
         row_revision = excluded.row_revision,
         name = excluded.name,
         icon = excluded.icon,
         icon_color = excluded.icon_color,
         "order" = excluded."order""#,
                params![ws.id, ws.name, ws.icon, ws.icon_color, num(ws.order), rev],
            )?;
        }

        let session_groups = config.session_groups.unwrap_or_default();
        let group_ids: Vec<&str> = session_groups.iter().map(|g| g.id.as_str()).collect();
        prune_missing(&tx, "session_groups", "id", &group_ids, base)?;
        for g in &session_groups {
            tx.execute(
                r#"INSERT INTO session_groups (id, name, icon, icon_color, "order", workspace_id, row_revision)
       VALUES (?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(id) DO UPDATE SET
         row_revision = excluded.row_revision,
         name = excluded.name,
         icon = excluded.icon,
         icon_color = excluded.icon_color,
         "order" = excluded."order",
         workspace_id = excluded.workspace_id"#,
                params![
                    g.id,
                    g.name,
                    g.icon,
                    g.icon_color,
                    num(g.order),
                    g.workspace_id,
                    rev,
                ],
            )?;
        }

        tx.execute(
            "INSERT OR REPLACE INTO schema_meta (key, value) VALUES (?, ?)",
            params![CONFIG_REVISION_KEY, format_js_number(revision)],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Deletes the rows of `table` whose key is not in `keep` and that the
/// saving client could have seen (`row_revision <= base`). A row written by
/// a later save is one this client never loaded, not one it deleted.
///
/// `table` and `key_column` are literals at every call site; SQLite cannot
/// bind identifiers.
fn prune_missing(
    tx: &Transaction<'_>,
    table: &str,
    key_column: &str,
    keep: &[&str],
    base: f64,
) -> Result<()> {
    let wanted: HashSet<&str> = keep.iter().copied().collect();
    let existing = {
        let mut stmt = tx.prepare(&format!(
            "SELECT {key_column} AS key, row_revision AS revision FROM {table}"
        ))?;
        let mut rows = stmt.query([])?;
        let mut existing = Vec::new();
        while let Some(row) = rows.next()? {
            existing.push((row.get::<_, SqlValue>("key")?, get_f64(row, "revision")?));
        }
        existing
    };
    let delete = format!("DELETE FROM {table} WHERE {key_column} = ?");
    for (key, revision) in existing {
        // Only a text key can match: the snapshot's keys are strings, as the
        // `Set` of strings is in the TypeScript.
        if matches!(&key, SqlValue::Text(k) if wanted.contains(k.as_str())) {
            continue;
        }
        if revision > base {
            continue;
        }
        tx.prepare_cached(&delete)?.execute([key])?;
    }
    Ok(())
}

/// The first column of every row `sql` returns, as stored.
fn keys(conn: &Connection, sql: &str) -> Result<Vec<SqlValue>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row.get::<_, SqlValue>(0)?);
    }
    Ok(out)
}

fn collect<T>(conn: &Connection, sql: &str, map: impl Fn(&Row<'_>) -> Result<T>) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map(row)?);
    }
    Ok(out)
}

/// `readConfigRevision`: `Number(value)`, and 0 when that is not finite
/// (no row, or text that is not a number).
fn read_config_revision(conn: &Connection) -> Result<f64> {
    let mut stmt = conn.prepare("SELECT value FROM schema_meta WHERE key = ?")?;
    let mut rows = stmt.query([CONFIG_REVISION_KEY])?;
    let parsed = match rows.next()? {
        Some(row) => js_number(&get_text(row, "value")?),
        None => f64::NAN,
    };
    Ok(if parsed.is_finite() { parsed } else { 0.0 })
}

/// JavaScript's `Number(text)`: trimmed, empty is 0, `0x`/`0o`/`0b` prefixes
/// read in their base, anything else that is not a decimal is NaN. Rust's
/// parser also takes `inf` and `nan`, which JavaScript reads as NaN; both are
/// non-finite, which is all a caller here looks at.
fn js_number(text: &str) -> f64 {
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if text.is_empty() {
        return 0.0;
    }
    let radix = match text.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = &text[2..];
        if digits.is_empty() {
            return f64::NAN;
        }
        return digits
            .chars()
            .try_fold(0.0_f64, |acc, c| {
                c.to_digit(radix)
                    .map(|d| acc * f64::from(radix) + f64::from(d))
            })
            .unwrap_or(f64::NAN);
    }
    text.parse().unwrap_or(f64::NAN)
}

/// A number as `JSON.stringify` writes it: integral values without a
/// fraction, so a revision reads back as `3`, not `3.0`.
fn js_number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() <= NO_BASE_REVISION {
        // In range and integral: the cast is exact.
        Value::from(n as i64)
    } else {
        Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

/// An object's keys in the order JavaScript enumerates them: array-index
/// keys first, ascending, then the rest in insertion order.
fn js_key_order(map: Map<String, Value>) -> Map<String, Value> {
    let index = |key: &str| -> Option<u32> {
        key.parse::<u32>()
            .ok()
            .filter(|n| *n != u32::MAX && n.to_string() == key)
    };
    if !map.keys().any(|k| index(k).is_some()) {
        return map;
    }
    let (mut indexed, rest): (Vec<_>, Vec<_>) =
        map.into_iter().partition(|(k, _)| index(k).is_some());
    indexed.sort_by_key(|(k, _)| index(k));
    indexed.into_iter().chain(rest).collect()
}

/// `loadAgentCommands`: by agent type, `args` parsed, the optional fields
/// present only when their column is not NULL.
pub(crate) fn load_agent_commands(conn: &Connection) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare("SELECT * FROM agent_commands")?;
    let mut rows = stmt.query([])?;
    let mut result = Map::new();
    while let Some(row) = rows.next()? {
        let cmd = AgentCommandConfig {
            command: get_text(row, "command")?,
            args: parse_json(&get_text(row, "args")?)?,
            headless_args: get_opt_text(row, "headless_args")?
                .map(|t| parse_json(&t))
                .transpose()?,
            fallback_command: get_opt_text(row, "fallback_command")?,
            fallback_args: get_opt_text(row, "fallback_args")?
                .map(|t| parse_json(&t))
                .transpose()?,
        };
        result.insert(get_text(row, "agent_type")?, serde_json::to_value(cmd)?);
    }
    Ok(js_key_order(result))
}

/// A `remote_hosts` row as `loadRemoteHosts` maps it.
pub(crate) fn row_to_remote_host(row: &Row<'_>) -> Result<RemoteHost> {
    Ok(RemoteHost {
        id: get_text(row, "id")?,
        label: get_text(row, "label")?,
        hostname: get_text(row, "hostname")?,
        user: get_text(row, "user")?,
        port: get_f64(row, "port")?,
        auth_method: get_opt_text(row, "auth_method")?.map(AuthMethod),
        ssh_key_path: get_opt_text(row, "ssh_key_path")?,
        credential_id: get_opt_text(row, "credential_id")?,
        encrypted_password: get_opt_text(row, "encrypted_password")?,
        ssh_options: get_opt_text(row, "ssh_options")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use serde_json::json;

    fn snapshot(revision: Option<i64>) -> Value {
        let mut config = json!({
            "version": 1,
            "defaults": { "shell": "/bin/bash", "fontSize": 14, "theme": "light" },
            "projects": [{ "name": "p", "path": "/p", "preferredAgents": ["claude"], "hostIds": ["h"] }],
            "agentCommands": { "claude": { "command": "claude", "args": ["--x"], "fallbackCommand": "c2" } },
            "workflows": [{
                "id": "w", "name": "W", "icon": "Zap", "iconColor": "#fff",
                "nodes": [], "edges": [], "enabled": false, "staggerDelayMs": 250
            }],
            "remoteHosts": [{ "id": "h", "label": "H", "hostname": "h.local", "user": "me", "port": 22 }],
            "tasks": [{
                "id": "t", "projectName": "p", "title": "T", "description": "d",
                "status": "todo", "order": 1, "createdAt": "c", "updatedAt": "u", "useWorktree": true
            }],
            "workspaces": [
                { "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#6b7280", "order": 0 },
                { "id": "w2", "name": "Work", "order": 1 }
            ],
            "sessionGroups": [{ "id": "g", "name": "G", "order": 0, "workspaceId": "w2" }]
        });
        if let Some(revision) = revision {
            config["revision"] = json!(revision);
        }
        config
    }

    fn task_ids(store: &Store) -> Vec<String> {
        store
            .db_list_tasks(None, None)
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect()
    }

    #[test]
    fn a_fresh_database_loads_the_defaults() {
        let store = test_support::store();
        let config = store.load_config().unwrap();
        let keys: Vec<_> = config.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "version",
                "revision",
                "defaults",
                "projects",
                "agentCommands",
                "workflows",
                "remoteHosts",
                "tasks",
                "workspaces",
                "sessionGroups"
            ]
        );
        assert_eq!(config["revision"], json!(0));
        assert_eq!(
            config["defaults"],
            json!({
                "shell": "/bin/zsh",
                "fontSize": 13,
                "theme": "dark",
                "reopenSessions": true,
                "startAtLogin": false,
                "domBlockRendering": true,
                "minimalShellPrompt": true,
                "keepSessionsRunning": true
            })
        );
        assert_eq!(
            config["agentCommands"],
            json!({ "claude": { "command": "claude", "args": [] } })
        );
        assert_eq!(config["workspaces"][0]["id"], json!("personal"));
    }

    #[test]
    fn a_saved_snapshot_loads_back() {
        let mut store = test_support::store();
        let saved = snapshot(None);
        store.save_config(&saved, &[]).unwrap();
        let loaded = store.load_config().unwrap();
        assert_eq!(loaded["revision"], json!(1));
        assert_eq!(loaded["defaults"]["shell"], json!("/bin/bash"));
        assert_eq!(loaded["defaults"]["fontSize"], json!(14));
        assert_eq!(
            loaded["projects"],
            json!([{ "name": "p", "path": "/p", "preferredAgents": ["claude"], "hostIds": ["h"], "workspaceId": "personal" }])
        );
        assert_eq!(loaded["agentCommands"], saved["agentCommands"]);
        assert_eq!(
            loaded["workflows"],
            json!([{
                "id": "w", "name": "W", "icon": "Zap", "iconColor": "#fff",
                "nodes": [], "edges": [], "enabled": false, "staggerDelayMs": 250.0,
                "workspaceId": "personal"
            }])
        );
        assert_eq!(
            loaded["remoteHosts"],
            json!([{ "id": "h", "label": "H", "hostname": "h.local", "user": "me", "port": 22.0, "authMethod": "agent" }])
        );
        assert_eq!(loaded["tasks"][0]["useWorktree"], json!(true));
        assert_eq!(
            loaded["workspaces"],
            json!([
                { "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#6b7280", "order": 0.0 },
                { "id": "w2", "name": "Work", "order": 1.0 }
            ])
        );
        assert_eq!(loaded["sessionGroups"][0]["workspaceId"], json!("w2"));

        // Saving what was loaded changes nothing but the revision.
        store.save_config(&loaded, &[]).unwrap();
        let again = store.load_config().unwrap();
        assert_eq!(again["revision"], json!(2));
        assert_eq!(again["tasks"], loaded["tasks"]);
        assert_eq!(again["projects"], loaded["projects"]);
    }

    #[test]
    fn rows_from_a_later_save_survive_an_older_snapshot() {
        let mut store = test_support::store();
        store.save_config(&snapshot(Some(0)), &[]).unwrap(); // revision 1

        // Another client, based on revision 1, adds a task: revision 2.
        let mut other = snapshot(Some(1));
        other["tasks"].as_array_mut().unwrap().push(json!({
            "id": "t2", "projectName": "p", "title": "T2", "description": "",
            "status": "todo", "order": 2, "createdAt": "c", "updatedAt": "u"
        }));
        store.save_config(&other, &[]).unwrap();

        // The first client, still at revision 1, saves without t2: kept.
        store.save_config(&snapshot(Some(1)), &[]).unwrap();
        assert_eq!(task_ids(&store), ["t", "t2"]);

        // A client that has seen t2's revision and leaves it out deleted it.
        store.save_config(&snapshot(Some(2)), &[]).unwrap();
        assert_eq!(task_ids(&store), ["t"]);

        // No base revision prunes everything the snapshot omits.
        let mut bare = snapshot(None);
        bare["tasks"] = json!([]);
        store.save_config(&bare, &[]).unwrap();
        assert!(task_ids(&store).is_empty());
    }

    #[test]
    fn workflows_are_pruned_without_a_revision_check() {
        let mut store = test_support::store();
        store.save_config(&snapshot(Some(0)), &[]).unwrap();
        let mut without = snapshot(Some(0));
        without["workflows"] = Value::Null;
        store.save_config(&without, &[]).unwrap();
        assert!(store.db_list_workflows().unwrap().is_empty());
    }

    #[test]
    fn deleted_defaults_go_and_absent_ones_stay() {
        let mut store = test_support::store();
        let mut config = snapshot(None);
        config["defaults"] = json!({ "rowHeight": 20, "serverPort": 4000, "theme": "light" });
        store.save_config(&config, &[]).unwrap();

        config["defaults"] = json!({ "theme": "dark" });
        store
            .save_config(&config, &["rowHeight".to_owned()])
            .unwrap();
        let defaults = &store.load_config().unwrap()["defaults"];
        assert!(defaults.get("rowHeight").is_none(), "{defaults}");
        assert_eq!(defaults["serverPort"], json!(4000));
        assert_eq!(defaults["theme"], json!("dark"));
    }

    #[test]
    fn defaults_load_as_load_defaults_builds_them() {
        let store = test_support::store();
        for (key, value) in [
            ("shell", "null"),
            ("rowHeight", "null"),
            ("envPassthrough", r#"["A", 1, "B"]"#),
            ("experimental", r#"{"b": true, "x": "yes", "2": false}"#),
            ("unlisted", "1"),
        ] {
            store
                .conn()
                .execute(
                    "INSERT INTO defaults (key, value) VALUES (?, ?)",
                    [key, value],
                )
                .unwrap();
        }
        let defaults = store.load_config().unwrap()["defaults"].clone();
        assert_eq!(defaults["shell"], json!("/bin/zsh"));
        assert_eq!(defaults["rowHeight"], Value::Null);
        assert_eq!(defaults["envPassthrough"], json!(["A", "B"]));
        let experimental = defaults["experimental"].as_object().unwrap();
        let keys: Vec<_> = experimental.keys().cloned().collect();
        assert_eq!(keys, ["2", "b"]);
        assert!(defaults.get("unlisted").is_none());
    }

    #[test]
    fn a_null_agent_command_is_kept_but_not_written() {
        let mut store = test_support::store();
        store.save_config(&snapshot(None), &[]).unwrap();
        let mut config = snapshot(None);
        config["agentCommands"] =
            json!({ "claude": null, "codex": { "command": "codex", "args": [] } });
        store.save_config(&config, &[]).unwrap();
        let commands = &store.load_config().unwrap()["agentCommands"];
        assert_eq!(commands["claude"]["command"], json!("claude"));
        assert_eq!(commands["codex"]["command"], json!("codex"));
    }

    #[test]
    fn missing_workspaces_mean_the_default_one() {
        let mut store = test_support::store();
        let mut config = snapshot(None);
        config.as_object_mut().unwrap().remove("workspaces");
        store.save_config(&config, &[]).unwrap();
        let ids: Vec<_> = store
            .db_list_workspaces()
            .unwrap()
            .into_iter()
            .map(|w| w.id)
            .collect();
        assert_eq!(ids, ["personal"]);
    }

    #[test]
    fn a_malformed_snapshot_is_a_json_error_and_writes_nothing() {
        let mut store = test_support::store();
        let config = json!({ "defaults": {}, "projects": [{ "name": 1 }] });
        assert!(matches!(
            store.save_config(&config, &[]),
            Err(crate::Error::Json(_))
        ));
        assert_eq!(store.load_config().unwrap()["revision"], json!(0));
    }

    #[test]
    fn revisions_read_as_number_reads_them() {
        assert_eq!(js_number(" 12 "), 12.0);
        assert_eq!(js_number(""), 0.0);
        assert_eq!(js_number("0x10"), 16.0);
        assert!(js_number("abc").is_nan());
        assert!(js_number("Infinity").is_infinite());
        assert_eq!(js_number_value(3.0), json!(3));
        assert_eq!(js_number_value(1.5), json!(1.5));

        let store = test_support::store();
        store
            .conn()
            .execute(
                "INSERT OR REPLACE INTO schema_meta (key, value) VALUES (?, 'garbage')",
                [CONFIG_REVISION_KEY],
            )
            .unwrap();
        assert_eq!(read_config_revision(store.conn()).unwrap(), 0.0);
    }
}

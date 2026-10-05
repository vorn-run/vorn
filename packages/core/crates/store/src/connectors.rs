//! Connectors: source connections, the durable connector inbox (poll pages,
//! webhook events, leases, retries) and the links from tasks to the items
//! they were imported from.

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Params, Row, Statement};
use serde_json::{Map, Value};
use vorn_protocol::{
    ConnectorInboxClaim, ConnectorInboxItem, ConnectorInboxRetry, ConnectorPollError,
    ConnectorPollPage, SourceConnection, TaskSourceLink, WebhookEvent,
};

use crate::sql::{
    get_f64, get_opt_text, get_text, iso_from_millis, json_text, num, parse_iso_millis, parse_json,
    random_uuid,
};
use crate::tasks::bind;
use crate::{Error, Result, Store};

/// Retries stop here: a row this old is failing for a reason a retry won't fix.
pub const MAX_INBOX_ATTEMPTS: i64 = 8;

/// The longest a failed inbox row waits before it is tried again, in ms.
const MAX_RETRY_DELAY_MS: f64 = 60.0 * 60_000.0;

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

/// A `source_connections` row as `rowToSourceConnection` maps it: NULL
/// columns leave their field out.
fn row_to_source_connection(row: &Row<'_>) -> Result<SourceConnection> {
    Ok(SourceConnection {
        id: get_text(row, "id")?,
        connector_id: get_text(row, "connector_id")?,
        name: get_text(row, "name")?,
        filters: parse_json(&get_text(row, "filters")?)?,
        sync_interval_minutes: get_f64(row, "sync_interval_minutes")?,
        status_mapping: parse_json(&get_text(row, "status_mapping")?)?,
        execution_project: get_opt_text(row, "execution_project")?,
        last_sync_at: get_opt_text(row, "last_sync_at")?,
        last_sync_error: get_opt_text(row, "last_sync_error")?,
        sync_cursor: get_opt_text(row, "sync_cursor")?,
        created_at: get_text(row, "created_at")?,
        signed_in_as: get_opt_text(row, "signed_in_as")?,
        signed_in_at: get_opt_text(row, "signed_in_at")?,
    })
}

/// A leased `connector_inbox` row; `connectorItem` is its parsed payload.
fn row_to_connector_inbox_item(row: &Row<'_>) -> Result<ConnectorInboxItem> {
    Ok(ConnectorInboxItem {
        id: get_f64(row, "id")?,
        lease_token: get_text(row, "lease_token")?,
        workflow_id: get_text(row, "workflow_id")?,
        connection_id: get_text(row, "connection_id")?,
        connector_id: get_text(row, "connector_id")?,
        event_id: get_text(row, "event_id")?,
        event_type: get_text(row, "event_type")?,
        event_timestamp: get_text(row, "event_timestamp")?,
        connector_item: parse_json(&get_text(row, "payload")?)?,
        attempts: get_f64(row, "attempts")?,
    })
}

fn row_to_task_source_link(row: &Row<'_>) -> Result<TaskSourceLink> {
    Ok(TaskSourceLink {
        task_id: get_text(row, "task_id")?,
        connection_id: get_text(row, "connection_id")?,
        connector_id: get_text(row, "connector_id")?,
        external_id: get_text(row, "external_id")?,
        external_url: get_text(row, "external_url")?,
        source_status_raw: get_text(row, "source_status_raw")?,
        source_updated_at: get_text(row, "source_updated_at")?,
        last_synced_at: get_text(row, "last_synced_at")?,
        conflict_state: get_text(row, "conflict_state")?,
    })
}

/// How long a row that has been tried `attempts` times waits before the next
/// try: a minute, doubling per attempt, capped at an hour.
fn retry_delay_ms(attempts: f64) -> f64 {
    (60_000.0 * 2f64.powf((attempts - 1.0).max(0.0))).min(MAX_RETRY_DELAY_MS)
}

impl Store {
    // ---- Source connections ------------------------------------------------

    /// `if (connectorId)`: an empty string filters nothing.
    pub fn db_list_source_connections(
        &self,
        connector_id: Option<&str>,
    ) -> Result<Vec<SourceConnection>> {
        match connector_id.filter(|c| !c.is_empty()) {
            Some(connector_id) => {
                let mut stmt = self
                    .conn()
                    .prepare("SELECT * FROM source_connections WHERE connector_id = ?")?;
                collect(&mut stmt, [connector_id], row_to_source_connection)
            }
            None => {
                let mut stmt = self.conn().prepare("SELECT * FROM source_connections")?;
                collect(&mut stmt, [], row_to_source_connection)
            }
        }
    }

    pub fn db_get_source_connection(&self, id: &str) -> Result<Option<SourceConnection>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT * FROM source_connections WHERE id = ?")?;
        let mut rows = stmt.query([id])?;
        rows.next()?.map(row_to_source_connection).transpose()
    }

    pub fn db_insert_source_connection(&self, conn: &SourceConnection) -> Result<()> {
        self.conn().execute(
            "INSERT INTO source_connections (id, connector_id, name, filters, sync_interval_minutes, status_mapping, execution_project, last_sync_at, last_sync_error, sync_cursor, created_at, signed_in_as, signed_in_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                conn.id,
                conn.connector_id,
                conn.name,
                json_text(&conn.filters)?,
                num(conn.sync_interval_minutes),
                json_text(&conn.status_mapping)?,
                conn.execution_project,
                conn.last_sync_at,
                conn.last_sync_error,
                conn.sync_cursor,
                conn.created_at,
                conn.signed_in_as,
                conn.signed_in_at,
            ],
        )?;
        Ok(())
    }

    /// `updates` is the partial connection as JSON. `lastSyncAt`,
    /// `lastSyncError` and `syncCursor` use an `in` test there, which also
    /// counts an explicit `undefined`; `present` names the keys the caller's
    /// object had, so those clear as they do in TypeScript.
    pub fn db_update_source_connection(
        &self,
        id: &str,
        updates: &Map<String, Value>,
        present: &[String],
    ) -> Result<()> {
        let mut sets: Vec<&str> = Vec::new();
        let mut args: Vec<SqlValue> = Vec::new();
        if let Some(value) = updates.get("name") {
            sets.push("name = ?");
            args.push(bind(value));
        }
        if let Some(value) = updates.get("filters") {
            sets.push("filters = ?");
            args.push(SqlValue::Text(json_text(value)?));
        }
        if let Some(value) = updates.get("syncIntervalMinutes") {
            sets.push("sync_interval_minutes = ?");
            args.push(bind(value));
        }
        if let Some(value) = updates.get("statusMapping") {
            sets.push("status_mapping = ?");
            args.push(SqlValue::Text(json_text(value)?));
        }
        if let Some(value) = updates.get("executionProject") {
            sets.push("execution_project = ?");
            args.push(bind(value));
        }
        for (key, column) in [
            ("lastSyncAt", "last_sync_at = ?"),
            ("lastSyncError", "last_sync_error = ?"),
            ("syncCursor", "sync_cursor = ?"),
        ] {
            if updates.contains_key(key) || present.iter().any(|k| k == key) {
                sets.push(column);
                args.push(updates.get(key).map_or(SqlValue::Null, bind));
            }
        }
        if sets.is_empty() {
            return Ok(());
        }
        args.push(SqlValue::Text(id.to_owned()));
        self.conn().execute(
            &format!(
                "UPDATE source_connections SET {} WHERE id = ?",
                sets.join(", ")
            ),
            params_from_iter(args),
        )?;
        Ok(())
    }

    /// Who a connection's window is signed in as; both `None` once it is
    /// signed out.
    pub fn db_set_connection_sign_in(
        &self,
        id: &str,
        signed_in_as: Option<&str>,
        signed_in_at: Option<&str>,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE source_connections SET signed_in_as = ?, signed_in_at = ? WHERE id = ?",
            params![signed_in_as, signed_in_at, id],
        )?;
        Ok(())
    }

    pub fn db_delete_source_connection(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM source_connections WHERE id = ?", [id])?;
        Ok(())
    }

    // ---- Durable connector inbox -------------------------------------------

    /// The cursor the workflow's last poll of `connection_id` stopped at.
    pub fn db_get_connector_poll_cursor(
        &self,
        workflow_id: &str,
        connection_id: &str,
    ) -> Result<Option<String>> {
        let mut stmt = self.conn().prepare(
            "SELECT cursor FROM connector_poll_state WHERE workflow_id = ? AND connection_id = ?",
        )?;
        let mut rows = stmt.query([workflow_id, connection_id])?;
        match rows.next()? {
            Some(row) => get_opt_text(row, "cursor"),
            None => Ok(None),
        }
    }

    pub fn db_count_active_connector_inbox_leases(&self, now: &str) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) AS count
       FROM connector_inbox
       WHERE status = 'leased' AND lease_until > ?",
            [now],
            |row| row.get(0),
        )?)
    }

    /// One webhook request becomes one durable inbox row, under the internal
    /// `webhook` connection every webhook event shares.
    pub fn db_enqueue_webhook_event(&self, args: &WebhookEvent) -> Result<()> {
        let d = self.conn();
        d.execute(
            "INSERT OR IGNORE INTO source_connections (id, connector_id, name, created_at)
     VALUES ('webhook', 'webhook', 'Webhook', ?)",
            [&args.received_at],
        )?;
        d.execute(
            "INSERT OR IGNORE INTO connector_inbox (
      workflow_id, connection_id, connector_id, event_id, event_type,
      event_timestamp, payload, status, attempts, available_at, created_at
    ) VALUES (?, 'webhook', 'webhook', ?, 'webhook', ?, ?, 'pending', 0, ?, ?)",
            params![
                args.workflow_id,
                args.event_id,
                args.received_at,
                json_text(&args.item)?,
                args.received_at,
                args.received_at,
            ],
        )?;
        Ok(())
    }

    /// Persists one remote page and its checkpoint in one transaction, so a
    /// crash never leaves a cursor past events that were only in memory.
    /// Answers how many events were new.
    pub fn db_record_connector_poll_page(&mut self, args: &ConnectorPollPage) -> Result<i64> {
        let tx = self.conn_mut().transaction()?;
        let mut inserted = 0_i64;
        {
            let mut insert = tx.prepare(
                "
      INSERT OR IGNORE INTO connector_inbox (
        workflow_id, connection_id, connector_id, event_id, event_type,
        event_timestamp, payload, status, attempts, available_at, created_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', 0, ?, ?)
    ",
            )?;
            for event in &args.events {
                let changes = insert.execute(params![
                    args.workflow_id,
                    args.connection_id,
                    args.connector_id,
                    event.event_id,
                    event.event_type,
                    event.event_timestamp,
                    json_text(&event.connector_item)?,
                    args.polled_at,
                    args.polled_at,
                ])?;
                inserted += i64::try_from(changes).unwrap_or(i64::MAX);
            }
        }
        tx.execute(
            "INSERT INTO connector_poll_state (
         workflow_id, connection_id, cursor, last_polled_at, last_error
       ) VALUES (?, ?, ?, ?, NULL)
       ON CONFLICT(workflow_id) DO UPDATE SET
         connection_id = excluded.connection_id,
         cursor = excluded.cursor,
         last_polled_at = excluded.last_polled_at,
         last_error = NULL",
            params![
                args.workflow_id,
                args.connection_id,
                args.cursor,
                args.polled_at
            ],
        )?;
        // The connection-level fields stay current for the settings UI.
        tx.execute(
            "UPDATE source_connections
       SET sync_cursor = ?, last_sync_at = ?, last_sync_error = NULL
       WHERE id = ?",
            params![args.cursor, args.polled_at, args.connection_id],
        )?;
        tx.commit()?;
        Ok(inserted)
    }

    /// Records a failed poll. The cursor is kept only when the workflow still
    /// polls the same connection.
    pub fn db_record_connector_poll_error(&mut self, args: &ConnectorPollError) -> Result<()> {
        let tx = self.conn_mut().transaction()?;
        tx.execute(
            "INSERT INTO connector_poll_state (
         workflow_id, connection_id, cursor, last_polled_at, last_error
       ) VALUES (?, ?, NULL, ?, ?)
       ON CONFLICT(workflow_id) DO UPDATE SET
         connection_id = excluded.connection_id,
         cursor = CASE
           WHEN connector_poll_state.connection_id = excluded.connection_id
             THEN connector_poll_state.cursor
           ELSE NULL
         END,
         last_polled_at = excluded.last_polled_at,
         last_error = excluded.last_error",
            params![
                args.workflow_id,
                args.connection_id,
                args.polled_at,
                args.error
            ],
        )?;
        tx.execute(
            "UPDATE source_connections SET last_sync_at = ?, last_sync_error = ? WHERE id = ?",
            params![args.polled_at, args.error, args.connection_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Leases ready rows (pending and due, or leased with an expired lease)
    /// before they are broadcast. Each claim counts as an attempt.
    pub fn db_claim_connector_inbox(
        &mut self,
        args: &ConnectorInboxClaim,
    ) -> Result<Vec<ConnectorInboxItem>> {
        let tx = self.conn_mut().transaction()?;
        let mut claimed = Vec::new();
        {
            let mut select = tx.prepare(
                "SELECT id
         FROM connector_inbox
         WHERE (status = 'pending' AND available_at <= ?)
            OR (status = 'leased' AND lease_until <= ?)
         ORDER BY created_at, id
         LIMIT ?",
            )?;
            let ids = collect(
                &mut select,
                params![args.now, args.now, num(args.limit)],
                |row| Ok(row.get::<_, i64>("id")?),
            )?;
            // Claimability is re-checked inside the UPDATE so a concurrent
            // claimer cannot steal a lease it lost the race for.
            let mut claim = tx.prepare(
                "UPDATE connector_inbox
       SET status = 'leased', attempts = attempts + 1, lease_until = ?, lease_token = ?
       WHERE id = ?
         AND ((status = 'pending' AND available_at <= ?)
           OR (status = 'leased' AND lease_until <= ?))",
            )?;
            let mut read = tx.prepare(
                "SELECT id, workflow_id, connection_id, connector_id, event_id,
              event_type, event_timestamp, payload, attempts, lease_token
       FROM connector_inbox WHERE id = ?",
            )?;
            for id in ids {
                let changes = claim.execute(params![
                    args.lease_until,
                    random_uuid(),
                    id,
                    args.now,
                    args.now
                ])?;
                if changes != 1 {
                    continue;
                }
                let mut rows = read.query([id])?;
                if let Some(row) = rows.next()? {
                    claimed.push(row_to_connector_inbox_item(row)?);
                }
            }
        }
        tx.commit()?;
        Ok(claimed)
    }

    /// Marks a leased row processed; false when the lease is no longer ours.
    pub fn db_complete_connector_inbox(
        &self,
        id: i64,
        lease_token: &str,
        processed_at: &str,
    ) -> Result<bool> {
        let changes = self.conn().execute(
            "UPDATE connector_inbox
       SET status = 'processed', processed_at = ?, lease_until = NULL,
           lease_token = NULL, last_error = NULL
       WHERE id = ? AND status = 'leased' AND lease_token = ?",
            params![processed_at, id, lease_token],
        )?;
        Ok(changes == 1)
    }

    /// Puts a failed row back with backoff, or marks it dead once it has had
    /// [`MAX_INBOX_ATTEMPTS`]. Either way the connection's `last_sync_error`
    /// names the workflow. Not one transaction, as in TypeScript.
    pub fn db_retry_connector_inbox(&self, args: &ConnectorInboxRetry) -> Result<bool> {
        let d = self.conn();
        let id = num(args.id);
        let row: Option<(f64, String)> = {
            let mut stmt = d.prepare(
                "SELECT attempts, workflow_id FROM connector_inbox
       WHERE id = ? AND status = 'leased' AND lease_token = ?",
            )?;
            let mut rows = stmt.query(params![id, args.lease_token])?;
            match rows.next()? {
                Some(row) => Some((get_f64(row, "attempts")?, get_text(row, "workflow_id")?)),
                None => None,
            }
        };
        let Some((attempts, workflow_id)) = row else {
            return Ok(false);
        };
        // Attributed to the workflow, since webhook rows share one connection.
        let attributed = format!("Workflow {workflow_id}: {}", args.error);
        if attempts >= MAX_INBOX_ATTEMPTS as f64 {
            let dead = d.execute(
                "UPDATE connector_inbox
         SET status = 'dead', lease_until = NULL, lease_token = NULL,
             last_error = ?, processed_at = ?
         WHERE id = ? AND status = 'leased' AND lease_token = ?",
                params![args.error, args.now, id, args.lease_token],
            )?;
            if dead != 1 {
                return Ok(false);
            }
            d.execute(
                "UPDATE source_connections
       SET last_sync_error = ?
       WHERE id = (SELECT connection_id FROM connector_inbox WHERE id = ?)",
                params![
                    format!("{attributed} (gave up after {MAX_INBOX_ATTEMPTS} attempts)"),
                    id
                ],
            )?;
            return Ok(true);
        }
        // `new Date(NaN).toISOString()` throws this in JavaScript.
        let now = parse_iso_millis(&args.now)
            .ok_or_else(|| Error::Refused("Invalid time value".into()))?;
        // The delay is whole milliseconds for whole attempts; `new Date`
        // truncates any fraction the same way.
        let available_at = iso_from_millis(now + retry_delay_ms(attempts) as i64);
        let changes = d.execute(
            "UPDATE connector_inbox
       SET status = 'pending', available_at = ?, lease_until = NULL,
           lease_token = NULL, last_error = ?
       WHERE id = ? AND status = 'leased' AND lease_token = ?",
            params![available_at, args.error, id, args.lease_token],
        )?;
        if changes != 1 {
            return Ok(false);
        }
        d.execute(
            "UPDATE source_connections
     SET last_sync_error = ?
     WHERE id = (SELECT connection_id FROM connector_inbox WHERE id = ?)",
            params![attributed, id],
        )?;
        Ok(true)
    }

    /// The renderer could not take this event yet (another run is parked at
    /// an approval gate). Not an attempt and not an error.
    pub fn db_defer_connector_inbox(
        &self,
        id: i64,
        lease_token: &str,
        available_at: &str,
    ) -> Result<bool> {
        let changes = self.conn().execute(
            "UPDATE connector_inbox
       SET status = 'pending',
           attempts = MAX(0, attempts - 1),
           available_at = ?,
           lease_until = NULL,
           lease_token = NULL
       WHERE id = ? AND status = 'leased' AND lease_token = ?",
            params![available_at, id, lease_token],
        )?;
        Ok(changes == 1)
    }

    pub fn db_renew_connector_inbox_lease(
        &self,
        id: i64,
        lease_token: &str,
        lease_until: &str,
    ) -> Result<bool> {
        let changes = self.conn().execute(
            "UPDATE connector_inbox
       SET lease_until = ?
       WHERE id = ? AND status = 'leased' AND lease_token = ?",
            params![lease_until, id, lease_token],
        )?;
        Ok(changes == 1)
    }

    /// A restart invalidates every in-memory workflow owner, so the previous
    /// process's leases become claimable at once.
    pub fn db_release_connector_inbox_leases(&self, now: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE connector_inbox
       SET status = 'pending', available_at = ?, lease_until = NULL, lease_token = NULL
       WHERE status = 'leased'",
            [now],
        )?;
        Ok(())
    }

    // ---- Task source links -------------------------------------------------

    pub fn db_get_task_source_link(&self, task_id: &str) -> Result<Option<TaskSourceLink>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT * FROM task_source_links WHERE task_id = ?")?;
        let mut rows = stmt.query([task_id])?;
        rows.next()?.map(row_to_task_source_link).transpose()
    }

    pub fn db_get_task_source_link_by_external_id(
        &self,
        connection_id: &str,
        external_id: &str,
    ) -> Result<Option<TaskSourceLink>> {
        let mut stmt = self.conn().prepare(
            "SELECT * FROM task_source_links WHERE connection_id = ? AND external_id = ?",
        )?;
        let mut rows = stmt.query([connection_id, external_id])?;
        rows.next()?.map(row_to_task_source_link).transpose()
    }

    pub fn db_list_task_source_links(&self, connection_id: &str) -> Result<Vec<TaskSourceLink>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT * FROM task_source_links WHERE connection_id = ?")?;
        collect(&mut stmt, [connection_id], row_to_task_source_link)
    }

    pub fn db_insert_task_source_link(&self, link: &TaskSourceLink) -> Result<()> {
        self.conn().execute(
            "INSERT INTO task_source_links (task_id, connection_id, connector_id, external_id, external_url, source_status_raw, source_updated_at, last_synced_at, conflict_state)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                link.task_id,
                link.connection_id,
                link.connector_id,
                link.external_id,
                link.external_url,
                link.source_status_raw,
                link.source_updated_at,
                link.last_synced_at,
                link.conflict_state,
            ],
        )?;
        Ok(())
    }

    /// `updates` is the partial link as JSON; only the sync fields change.
    pub fn db_update_task_source_link(
        &self,
        task_id: &str,
        updates: &Map<String, Value>,
    ) -> Result<()> {
        let mut sets: Vec<&str> = Vec::new();
        let mut args: Vec<SqlValue> = Vec::new();
        for (key, column) in [
            ("sourceStatusRaw", "source_status_raw = ?"),
            ("sourceUpdatedAt", "source_updated_at = ?"),
            ("lastSyncedAt", "last_synced_at = ?"),
            ("conflictState", "conflict_state = ?"),
        ] {
            if let Some(value) = updates.get(key) {
                sets.push(column);
                args.push(bind(value));
            }
        }
        if sets.is_empty() {
            return Ok(());
        }
        args.push(SqlValue::Text(task_id.to_owned()));
        self.conn().execute(
            &format!(
                "UPDATE task_source_links SET {} WHERE task_id = ?",
                sets.join(", ")
            ),
            params_from_iter(args),
        )?;
        Ok(())
    }

    pub fn db_delete_task_source_link(&self, task_id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM task_source_links WHERE task_id = ?", [task_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use serde_json::json;

    const T0: &str = "2026-10-05T00:00:00.000Z";

    fn connection(id: &str, connector_id: &str) -> SourceConnection {
        serde_json::from_value(json!({
            "id": id,
            "connectorId": connector_id,
            "name": "Conn",
            "filters": { "state": "open" },
            "syncIntervalMinutes": 5,
            "statusMapping": {},
            "createdAt": T0
        }))
        .unwrap()
    }

    fn workflow(store: &Store, id: &str) {
        store
            .conn()
            .execute(
                "INSERT INTO workflows (id, name, icon, icon_color) VALUES (?, 'W', 'i', '#000')",
                [id],
            )
            .unwrap();
    }

    fn page(
        workflow_id: &str,
        connection_id: &str,
        cursor: Option<&str>,
        events: &[&str],
    ) -> ConnectorPollPage {
        serde_json::from_value(json!({
            "workflowId": workflow_id,
            "connectionId": connection_id,
            "connectorId": "github",
            "cursor": cursor,
            "polledAt": T0,
            "events": events.iter().map(|id| json!({
                "eventId": id,
                "eventType": "created",
                "eventTimestamp": T0,
                "connectorItem": { "id": id }
            })).collect::<Vec<_>>()
        }))
        .unwrap()
    }

    fn claim(store: &mut Store, now: &str) -> Vec<ConnectorInboxItem> {
        store
            .db_claim_connector_inbox(&ConnectorInboxClaim {
                now: now.into(),
                lease_until: "2026-10-05T00:05:00.000Z".into(),
                limit: 10.0,
            })
            .unwrap()
    }

    fn inbox_row(store: &Store, id: f64) -> (String, i64, String, Option<String>) {
        store
            .conn()
            .query_row(
                "SELECT status, attempts, available_at, last_error FROM connector_inbox WHERE id = ?",
                [id as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
    }

    fn retry(store: &Store, item: &ConnectorInboxItem, now: &str) -> Result<bool> {
        store.db_retry_connector_inbox(&ConnectorInboxRetry {
            id: item.id,
            lease_token: item.lease_token.clone(),
            error: "boom".into(),
            now: now.into(),
        })
    }

    /// A store with workflow `w`, connection `c` and one polled event `e1`.
    fn inbox_store() -> Store {
        let mut store = test_support::store();
        workflow(&store, "w");
        store
            .db_insert_source_connection(&connection("c", "github"))
            .unwrap();
        let inserted = store
            .db_record_connector_poll_page(&page("w", "c", Some("cur1"), &["e1"]))
            .unwrap();
        assert_eq!(inserted, 1);
        store
    }

    #[test]
    fn source_connections_round_trip_and_update() {
        let store = test_support::store();
        store
            .db_insert_source_connection(&connection("a", "github"))
            .unwrap();
        store
            .db_insert_source_connection(&connection("b", "linear"))
            .unwrap();
        assert_eq!(store.db_list_source_connections(None).unwrap().len(), 2);
        assert_eq!(store.db_list_source_connections(Some("")).unwrap().len(), 2);
        let github = store.db_list_source_connections(Some("github")).unwrap();
        assert_eq!(github.len(), 1);
        assert_eq!(github[0].filters, json!({ "state": "open" }));

        let keys: Vec<_> = serde_json::to_value(&github[0])
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        for absent in [
            "executionProject",
            "lastSyncAt",
            "lastSyncError",
            "syncCursor",
            "signedInAs",
        ] {
            assert!(!keys.contains(&absent.to_string()), "{keys:?}");
        }

        let updates = json!({ "name": "N", "filters": [1], "syncIntervalMinutes": 15, "lastSyncAt": "x", "syncCursor": "c" });
        store
            .db_update_source_connection("a", updates.as_object().unwrap(), &[])
            .unwrap();
        let a = store.db_get_source_connection("a").unwrap().unwrap();
        assert_eq!(a.name, "N");
        assert_eq!(a.filters, json!([1]));
        assert_eq!(a.sync_interval_minutes, 15.0);
        assert_eq!(a.last_sync_at.as_deref(), Some("x"));

        // `{ lastSyncAt: undefined }` clears it; a missing key leaves syncCursor.
        store
            .db_update_source_connection("a", &Map::new(), &["lastSyncAt".into()])
            .unwrap();
        let a = store.db_get_source_connection("a").unwrap().unwrap();
        assert_eq!(a.last_sync_at, None);
        assert_eq!(a.sync_cursor.as_deref(), Some("c"));

        store
            .db_set_connection_sign_in("a", Some("me@x"), Some(T0))
            .unwrap();
        let a = store.db_get_source_connection("a").unwrap().unwrap();
        assert_eq!(a.signed_in_as.as_deref(), Some("me@x"));
        store.db_set_connection_sign_in("a", None, None).unwrap();
        assert_eq!(
            store
                .db_get_source_connection("a")
                .unwrap()
                .unwrap()
                .signed_in_at,
            None
        );

        store.db_delete_source_connection("a").unwrap();
        assert!(store.db_get_source_connection("a").unwrap().is_none());
    }

    #[test]
    fn poll_page_records_events_cursor_and_connection_state() {
        let mut store = inbox_store();
        assert_eq!(
            store
                .db_get_connector_poll_cursor("w", "c")
                .unwrap()
                .as_deref(),
            Some("cur1")
        );
        assert_eq!(
            store.db_get_connector_poll_cursor("w", "other").unwrap(),
            None
        );
        // A repeated event is ignored; a new one counts.
        let inserted = store
            .db_record_connector_poll_page(&page("w", "c", None, &["e1", "e2"]))
            .unwrap();
        assert_eq!(inserted, 1);
        assert_eq!(store.db_get_connector_poll_cursor("w", "c").unwrap(), None);
        let c = store.db_get_source_connection("c").unwrap().unwrap();
        assert_eq!(c.last_sync_at.as_deref(), Some(T0));
        assert_eq!(c.sync_cursor, None);
    }

    #[test]
    fn poll_error_keeps_the_cursor_only_for_the_same_connection() {
        let mut store = inbox_store();
        store
            .db_insert_source_connection(&connection("c2", "github"))
            .unwrap();
        let error = |connection_id: &str| ConnectorPollError {
            workflow_id: "w".into(),
            connection_id: connection_id.into(),
            error: "down".into(),
            polled_at: "2026-10-05T01:00:00.000Z".into(),
        };
        store.db_record_connector_poll_error(&error("c")).unwrap();
        assert_eq!(
            store
                .db_get_connector_poll_cursor("w", "c")
                .unwrap()
                .as_deref(),
            Some("cur1")
        );
        let c = store.db_get_source_connection("c").unwrap().unwrap();
        assert_eq!(c.last_sync_error.as_deref(), Some("down"));

        store.db_record_connector_poll_error(&error("c2")).unwrap();
        assert_eq!(store.db_get_connector_poll_cursor("w", "c2").unwrap(), None);
        assert_eq!(store.db_get_connector_poll_cursor("w", "c").unwrap(), None);
    }

    #[test]
    fn inbox_lease_renew_and_complete() {
        let mut store = inbox_store();
        let items = claim(&mut store, T0);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.attempts, 1.0);
        assert_eq!(item.connector_item, json!({ "id": "e1" }));
        assert_eq!(item.event_id, "e1");
        // Leased rows are not claimed again while the lease holds.
        assert!(claim(&mut store, T0).is_empty());
        assert_eq!(store.db_count_active_connector_inbox_leases(T0).unwrap(), 1);

        let id = item.id as i64;
        assert!(!store
            .db_renew_connector_inbox_lease(id, "wrong", "z")
            .unwrap());
        assert!(store
            .db_renew_connector_inbox_lease(id, &item.lease_token, "2026-10-05T00:10:00.000Z")
            .unwrap());
        // Past the original lease, but the renewed one still holds.
        assert!(claim(&mut store, "2026-10-05T00:06:00.000Z").is_empty());

        assert!(store
            .db_complete_connector_inbox(id, &item.lease_token, "2026-10-05T00:01:00.000Z")
            .unwrap());
        assert!(!store
            .db_complete_connector_inbox(id, &item.lease_token, "2026-10-05T00:01:00.000Z")
            .unwrap());
        assert_eq!(inbox_row(&store, item.id).0, "processed");
        assert_eq!(store.db_count_active_connector_inbox_leases(T0).unwrap(), 0);
    }

    #[test]
    fn expired_leases_are_reclaimed() {
        let mut store = inbox_store();
        let first = claim(&mut store, T0);
        let again = claim(&mut store, "2026-10-05T00:05:00.000Z");
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].attempts, 2.0);
        assert_ne!(again[0].lease_token, first[0].lease_token);
    }

    #[test]
    fn retry_backs_off_and_gives_up_after_the_last_attempt() {
        let mut store = inbox_store();
        let item = claim(&mut store, T0).remove(0);
        assert!(!store
            .db_retry_connector_inbox(&ConnectorInboxRetry {
                id: item.id,
                lease_token: "wrong".into(),
                error: "boom".into(),
                now: T0.into(),
            })
            .unwrap());
        assert!(retry(&store, &item, T0).unwrap());
        let (status, attempts, available_at, last_error) = inbox_row(&store, item.id);
        assert_eq!((status.as_str(), attempts), ("pending", 1));
        assert_eq!(available_at, "2026-10-05T00:01:00.000Z");
        assert_eq!(last_error.as_deref(), Some("boom"));
        let c = store.db_get_source_connection("c").unwrap().unwrap();
        assert_eq!(c.last_sync_error.as_deref(), Some("Workflow w: boom"));

        // Not due yet; then due, and the second retry waits two minutes.
        assert!(claim(&mut store, T0).is_empty());
        let item = claim(&mut store, &available_at).remove(0);
        assert_eq!(item.attempts, 2.0);
        assert!(retry(&store, &item, T0).unwrap());
        assert_eq!(inbox_row(&store, item.id).2, "2026-10-05T00:02:00.000Z");

        // Drive it to the last attempt, then it goes dead.
        store
            .conn()
            .execute(
                "UPDATE connector_inbox SET attempts = ?, available_at = ?",
                params![MAX_INBOX_ATTEMPTS - 1, T0],
            )
            .unwrap();
        let item = claim(&mut store, T0).remove(0);
        assert_eq!(item.attempts, MAX_INBOX_ATTEMPTS as f64);
        assert!(retry(&store, &item, "2026-10-05T03:00:00.000Z").unwrap());
        assert_eq!(inbox_row(&store, item.id).0, "dead");
        let c = store.db_get_source_connection("c").unwrap().unwrap();
        assert_eq!(
            c.last_sync_error.as_deref(),
            Some("Workflow w: boom (gave up after 8 attempts)")
        );
        assert!(claim(&mut store, "2027-01-01T00:00:00.000Z").is_empty());
    }

    #[test]
    fn retry_delay_caps_at_an_hour() {
        assert_eq!(retry_delay_ms(0.0), 60_000.0);
        assert_eq!(retry_delay_ms(1.0), 60_000.0);
        assert_eq!(retry_delay_ms(3.0), 240_000.0);
        assert_eq!(retry_delay_ms(7.0), 3_600_000.0);
    }

    #[test]
    fn retry_with_a_bad_time_refuses_as_javascript_throws() {
        let mut store = inbox_store();
        let item = claim(&mut store, T0).remove(0);
        let err = retry(&store, &item, "not a date").unwrap_err();
        assert_eq!(err.to_string(), "Invalid time value");
        assert_eq!(inbox_row(&store, item.id).0, "leased");
    }

    #[test]
    fn defer_returns_the_attempt_and_release_frees_leases() {
        let mut store = inbox_store();
        let item = claim(&mut store, T0).remove(0);
        assert!(store
            .db_defer_connector_inbox(
                item.id as i64,
                &item.lease_token,
                "2026-10-05T00:00:30.000Z"
            )
            .unwrap());
        let (status, attempts, available_at, _) = inbox_row(&store, item.id);
        assert_eq!((status.as_str(), attempts), ("pending", 0));
        assert_eq!(available_at, "2026-10-05T00:00:30.000Z");
        assert!(!store
            .db_defer_connector_inbox(item.id as i64, &item.lease_token, T0)
            .unwrap());

        let item = claim(&mut store, "2026-10-05T00:00:30.000Z").remove(0);
        store
            .db_release_connector_inbox_leases("2026-10-05T00:00:40.000Z")
            .unwrap();
        let (status, attempts, available_at, _) = inbox_row(&store, item.id);
        assert_eq!((status.as_str(), attempts), ("pending", 1));
        assert_eq!(available_at, "2026-10-05T00:00:40.000Z");
        assert_eq!(claim(&mut store, "2026-10-05T00:00:40.000Z").len(), 1);
    }

    #[test]
    fn webhook_events_enqueue_once_under_the_shared_connection() {
        let mut store = test_support::store();
        workflow(&store, "w");
        let event = WebhookEvent {
            workflow_id: "w".into(),
            event_id: "h1".into(),
            received_at: T0.into(),
            item: json!({ "title": "hi" }),
        };
        store.db_enqueue_webhook_event(&event).unwrap();
        store.db_enqueue_webhook_event(&event).unwrap();
        let webhook = store.db_get_source_connection("webhook").unwrap().unwrap();
        assert_eq!(webhook.filters, json!({}));
        let items = claim(&mut store, T0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].connection_id, "webhook");
        assert_eq!(items[0].event_type, "webhook");
        assert_eq!(items[0].connector_item, json!({ "title": "hi" }));
    }

    #[test]
    fn task_source_links_round_trip_and_update() {
        let store = test_support::store();
        store
            .db_insert_source_connection(&connection("c", "github"))
            .unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO tasks (id, project_name, title, created_at, updated_at) VALUES ('t', 'p', 'T', ?, ?)",
                [T0, T0],
            )
            .unwrap();
        let link: TaskSourceLink = serde_json::from_value(json!({
            "taskId": "t",
            "connectionId": "c",
            "connectorId": "github",
            "externalId": "42",
            "externalUrl": "https://x/42",
            "sourceStatusRaw": "open",
            "sourceUpdatedAt": T0,
            "lastSyncedAt": T0,
            "conflictState": "none"
        }))
        .unwrap();
        store.db_insert_task_source_link(&link).unwrap();
        let as_json = |l: Option<TaskSourceLink>| serde_json::to_value(l).unwrap();
        assert_eq!(
            as_json(store.db_get_task_source_link("t").unwrap()),
            as_json(Some(link.clone()))
        );
        assert_eq!(
            as_json(
                store
                    .db_get_task_source_link_by_external_id("c", "42")
                    .unwrap()
            ),
            as_json(Some(link))
        );
        assert!(store
            .db_get_task_source_link_by_external_id("c", "43")
            .unwrap()
            .is_none());

        let updates = json!({ "sourceStatusRaw": "closed", "conflictState": "local", "externalId": "ignored" });
        store
            .db_update_task_source_link("t", updates.as_object().unwrap())
            .unwrap();
        store.db_update_task_source_link("t", &Map::new()).unwrap();
        let links = store.db_list_task_source_links("c").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].source_status_raw, "closed");
        assert_eq!(links[0].conflict_state, "local");
        assert_eq!(links[0].external_id, "42");

        store.db_delete_task_source_link("t").unwrap();
        assert!(store.db_get_task_source_link("t").unwrap().is_none());
    }
}

//! Artifacts: published pages, their numbered versions, and the comments
//! written on them (`insertArtifact`, `sendArtifactDrafts` and the rest).

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde_json::{Map, Value};
use vorn_protocol::{
    Artifact, ArtifactAuthor, ArtifactComment, ArtifactCommentFilter, ArtifactCommentState,
    ArtifactKind, ArtifactVersion, NewArtifact, NewArtifactComment,
};

use crate::sql::{
    get_f64, get_opt_text, get_text, json_if_truthy, now_iso, num, parse_json, random_uuid,
};
use crate::tasks::bind;
use crate::{Error, Result, Store};

/// An `artifacts` row as `mapArtifactRow` maps it: `sessionId` and
/// `projectName` are always there, null when unset.
fn map_artifact_row(row: &Row<'_>) -> Result<Artifact> {
    Ok(Artifact {
        id: get_text(row, "id")?,
        kind: ArtifactKind(get_text(row, "kind")?),
        title: get_text(row, "title")?,
        session_id: get_opt_text(row, "session_id")?,
        project_name: get_opt_text(row, "project_name")?,
        latest_version: get_f64(row, "latest_version")?,
        gate_run_id: get_opt_text(row, "gate_run_id")?,
        gate_node_id: get_opt_text(row, "gate_node_id")?,
        created_at: get_text(row, "created_at")?,
        updated_at: get_text(row, "updated_at")?,
    })
}

/// An `artifact_comments` row as `mapArtifactCommentRow` maps it: `anchor`
/// is parsed, and null when empty or NULL.
fn map_comment_row(row: &Row<'_>) -> Result<ArtifactComment> {
    let anchor = match get_opt_text(row, "anchor")? {
        Some(text) if !text.is_empty() => parse_json(&text)?,
        _ => Value::Null,
    };
    Ok(ArtifactComment {
        id: get_text(row, "id")?,
        artifact_id: get_text(row, "artifact_id")?,
        version: get_f64(row, "version")?,
        anchor,
        body: get_text(row, "body")?,
        state: ArtifactCommentState(get_text(row, "state")?),
        batch_id: get_opt_text(row, "batch_id")?,
        created_at: get_text(row, "created_at")?,
        updated_at: get_text(row, "updated_at")?,
        sent_at: get_opt_text(row, "sent_at")?,
    })
}

fn get_artifact(conn: &Connection, id: &str) -> Result<Option<Artifact>> {
    let mut stmt = conn.prepare("SELECT * FROM artifacts WHERE id = ?")?;
    let mut rows = stmt.query([id])?;
    rows.next()?.map(map_artifact_row).transpose()
}

fn get_comment(conn: &Connection, id: &str) -> Result<Option<ArtifactComment>> {
    let mut stmt = conn.prepare("SELECT * FROM artifact_comments WHERE id = ?")?;
    let mut rows = stmt.query([id])?;
    rows.next()?.map(map_comment_row).transpose()
}

/// `listArtifactComments`, on a connection or inside a transaction.
fn list_comments(
    conn: &Connection,
    artifact_id: &str,
    filter: &ArtifactCommentFilter,
) -> Result<Vec<ArtifactComment>> {
    let mut clauses = vec!["artifact_id = ?"];
    let mut args = vec![SqlValue::Text(artifact_id.to_owned())];
    if let Some(version) = filter.version {
        clauses.push("version = ?");
        args.push(num(version));
    }
    if let Some(state) = filter.state.as_ref().filter(|s| !s.is_empty()) {
        clauses.push("state = ?");
        args.push(SqlValue::Text(state.0.clone()));
    }
    if let Some(batch_id) = filter.batch_id.as_ref().filter(|b| !b.is_empty()) {
        clauses.push("batch_id = ?");
        args.push(SqlValue::Text(batch_id.clone()));
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM artifact_comments WHERE {} ORDER BY created_at",
        clauses.join(" AND ")
    ))?;
    collect(&mut stmt, params_from_iter(args), map_comment_row)
}

/// Every row of a query, mapped.
fn collect<T>(
    stmt: &mut rusqlite::Statement<'_>,
    args: impl rusqlite::Params,
    map: impl Fn(&Row<'_>) -> Result<T>,
) -> Result<Vec<T>> {
    let mut rows = stmt.query(args)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map(row)?);
    }
    Ok(out)
}

fn text_id(row: &Row<'_>) -> Result<String> {
    get_text(row, "id")
}

impl Store {
    /// Creates an artifact with no versions yet. Returns
    /// `{ artifact, token }`, the token unlocking its pages.
    pub fn insert_artifact(&self, fields: &NewArtifact) -> Result<Value> {
        let now = now_iso();
        let id = random_uuid();
        let token = random_uuid().replace('-', "");
        self.conn().execute(
            "INSERT INTO artifacts (id, kind, title, session_id, project_name, token, latest_version,
         gate_run_id, gate_node_id, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?)",
            params![
                id,
                fields.kind.0,
                fields.title,
                fields.session_id,
                fields.project_name,
                token,
                fields.gate_run_id,
                fields.gate_node_id,
                now,
                now,
            ],
        )?;
        let artifact = get_artifact(self.conn(), &id)?;
        let mut out = Map::new();
        out.insert("artifact".into(), serde_json::to_value(artifact)?);
        out.insert("token".into(), Value::String(token));
        Ok(Value::Object(out))
    }

    pub fn get_artifact(&self, id: &str) -> Result<Option<Artifact>> {
        get_artifact(self.conn(), id)
    }

    pub fn get_artifact_token(&self, id: &str) -> Result<Option<String>> {
        let token: Option<Option<String>> = self
            .conn()
            .query_row("SELECT token FROM artifacts WHERE id = ?", [id], |row| {
                Ok(get_opt_text(row, "token"))
            })
            .optional()?
            .transpose()?;
        Ok(token.flatten())
    }

    /// The newest first. A session and a project narrow it together with OR,
    /// as the TypeScript does; `limit` defaults to 50.
    pub fn list_artifacts(
        &self,
        filter: &vorn_protocol::ArtifactFilter,
        limit: Option<f64>,
    ) -> Result<Vec<Artifact>> {
        let mut clauses = Vec::new();
        let mut args = Vec::new();
        if let Some(session) = filter.session_id.as_ref().filter(|s| !s.is_empty()) {
            clauses.push("session_id = ?");
            args.push(SqlValue::Text(session.clone()));
        }
        if let Some(project) = filter.project_name.as_ref().filter(|p| !p.is_empty()) {
            clauses.push("project_name = ?");
            args.push(SqlValue::Text(project.clone()));
        }
        let filter_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" OR "))
        };
        args.push(num(limit.unwrap_or(50.0)));
        let mut stmt = self.conn().prepare(&format!(
            "SELECT * FROM artifacts {filter_sql}
               ORDER BY updated_at DESC LIMIT ?"
        ))?;
        collect(&mut stmt, params_from_iter(args), map_artifact_row)
    }

    /// The artifact a gate's review pages are kept as, one version per round.
    pub fn find_gate_artifact(&self, run_id: &str, node_id: &str) -> Result<Option<Artifact>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT * FROM artifacts WHERE gate_run_id = ? AND gate_node_id = ?")?;
        let mut rows = stmt.query([run_id, node_id])?;
        rows.next()?.map(map_artifact_row).transpose()
    }

    pub fn rename_artifact(&self, id: &str, title: &str) -> Result<()> {
        self.conn()
            .execute("UPDATE artifacts SET title = ? WHERE id = ?", [title, id])?;
        Ok(())
    }

    /// Numbers the next version and makes it the latest in one transaction,
    /// so two publishes cannot share a number.
    pub fn add_artifact_version(
        &mut self,
        artifact_id: &str,
        author: &str,
        answers_batch_id: Option<&str>,
    ) -> Result<ArtifactVersion> {
        let tx = self.conn_mut().transaction()?;
        let latest: Option<f64> = tx
            .query_row(
                "SELECT latest_version FROM artifacts WHERE id = ?",
                [artifact_id],
                |row| Ok(get_f64(row, "latest_version")),
            )
            .optional()?
            .transpose()?;
        let Some(latest) = latest else {
            return Err(Error::Refused(format!("Artifact not found: {artifact_id}")));
        };
        let version = latest + 1.0;
        let now = now_iso();
        tx.execute(
            "INSERT INTO artifact_versions (artifact_id, version, author, answers_batch_id, created_at)
       VALUES (?, ?, ?, ?, ?)",
            params![artifact_id, num(version), author, answers_batch_id, now],
        )?;
        tx.execute(
            "UPDATE artifacts SET latest_version = ?, updated_at = ? WHERE id = ?",
            params![num(version), now, artifact_id],
        )?;
        tx.commit()?;
        Ok(ArtifactVersion {
            artifact_id: artifact_id.to_owned(),
            version,
            author: ArtifactAuthor(author.to_owned()),
            answers_batch_id: answers_batch_id.map(str::to_owned),
            created_at: now,
        })
    }

    pub fn list_artifact_versions(&self, artifact_id: &str) -> Result<Vec<ArtifactVersion>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT * FROM artifact_versions WHERE artifact_id = ? ORDER BY version")?;
        collect(&mut stmt, [artifact_id], |row| {
            Ok(ArtifactVersion {
                artifact_id: get_text(row, "artifact_id")?,
                version: get_f64(row, "version")?,
                author: ArtifactAuthor(get_text(row, "author")?),
                answers_batch_id: get_opt_text(row, "answers_batch_id")?,
                created_at: get_text(row, "created_at")?,
            })
        })
    }

    /// The latest batch sent on this artifact that no version has answered.
    pub fn unanswered_batch_id(&self, artifact_id: &str) -> Result<Option<String>> {
        let batch: Option<Option<String>> = self
            .conn()
            .query_row(
                "SELECT batch_id FROM artifact_comments
       WHERE artifact_id = ? AND state = 'sent' AND batch_id IS NOT NULL
         AND batch_id NOT IN (
           SELECT answers_batch_id FROM artifact_versions
           WHERE artifact_id = ? AND answers_batch_id IS NOT NULL
         )
       ORDER BY sent_at DESC LIMIT 1",
                [artifact_id, artifact_id],
                |row| Ok(get_opt_text(row, "batch_id")),
            )
            .optional()?
            .transpose()?;
        Ok(batch.flatten())
    }

    pub fn list_artifact_comments(
        &self,
        artifact_id: &str,
        filter: &ArtifactCommentFilter,
    ) -> Result<Vec<ArtifactComment>> {
        list_comments(self.conn(), artifact_id, filter)
    }

    pub fn get_artifact_comment(&self, id: &str) -> Result<Option<ArtifactComment>> {
        get_comment(self.conn(), id)
    }

    /// Adds a draft comment.
    pub fn insert_artifact_comment(&self, fields: &NewArtifactComment) -> Result<ArtifactComment> {
        let now = now_iso();
        let id = random_uuid();
        self.conn().execute(
            "INSERT INTO artifact_comments (id, artifact_id, version, anchor, body, state, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?, 'draft', ?, ?)",
            params![
                id,
                fields.artifact_id,
                num(fields.version),
                json_if_truthy(Some(&fields.anchor))?,
                fields.body,
                now,
                now,
            ],
        )?;
        get_comment(self.conn(), &id)?.ok_or(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    /// Changes a draft's words or anchor: a missing `anchor` key keeps it, an
    /// explicit null clears it; a missing or null `body` keeps it. A sent
    /// comment is part of the record and is not changed (`None`).
    pub fn update_artifact_comment(
        &self,
        id: &str,
        change: &Map<String, Value>,
    ) -> Result<Option<ArtifactComment>> {
        let Some(current) = get_comment(self.conn(), id)? else {
            return Ok(None);
        };
        if current.state.0 != "draft" {
            return Ok(None);
        }
        let anchor = change.get("anchor").unwrap_or(&current.anchor);
        let body = match change.get("body") {
            Some(body) if !body.is_null() => bind(body),
            _ => SqlValue::Text(current.body.clone()),
        };
        self.conn().execute(
            "UPDATE artifact_comments SET body = ?, anchor = ?, updated_at = ? WHERE id = ?",
            params![body, json_if_truthy(Some(anchor))?, now_iso(), id],
        )?;
        get_comment(self.conn(), id)
    }

    /// Drops a draft. False when there was no draft by that id.
    pub fn delete_artifact_comment(&self, id: &str) -> Result<bool> {
        let changes = self.conn().execute(
            "DELETE FROM artifact_comments WHERE id = ? AND state = 'draft'",
            [id],
        )?;
        Ok(changes > 0)
    }

    /// Seals every draft on the artifact into one batch, returning
    /// `{ batchId, comments }`, or `None` when there were no drafts.
    pub fn send_artifact_drafts(&mut self, artifact_id: &str) -> Result<Option<Value>> {
        let tx = self.conn_mut().transaction()?;
        let batch_id = random_uuid();
        let changes = tx.execute(
            "UPDATE artifact_comments SET state = 'sent', batch_id = ?, sent_at = ?
         WHERE artifact_id = ? AND state = 'draft'",
            params![batch_id, now_iso(), artifact_id],
        )?;
        if changes == 0 {
            tx.commit()?;
            return Ok(None);
        }
        let filter = ArtifactCommentFilter {
            batch_id: Some(batch_id.clone()),
            ..ArtifactCommentFilter::default()
        };
        let comments = list_comments(&tx, artifact_id, &filter)?;
        tx.commit()?;
        let mut out = Map::new();
        out.insert("batchId".into(), Value::String(batch_id));
        out.insert("comments".into(), serde_json::to_value(comments)?);
        Ok(Some(Value::Object(out)))
    }

    /// Removes artifacts untouched since `cutoff`, returning their ids so
    /// their pages can go too.
    pub fn delete_artifacts_updated_before(&self, cutoff: &str) -> Result<Vec<String>> {
        let ids = {
            let mut stmt = self
                .conn()
                .prepare("SELECT id FROM artifacts WHERE updated_at < ?")?;
            collect(&mut stmt, [cutoff], text_id)?
        };
        self.conn()
            .execute("DELETE FROM artifacts WHERE updated_at < ?", [cutoff])?;
        Ok(ids)
    }

    pub fn list_artifact_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare("SELECT id FROM artifacts")?;
        collect(&mut stmt, [], text_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use serde_json::json;
    use vorn_protocol::ArtifactFilter;

    fn new_artifact(
        store: &Store,
        session: Option<&str>,
        project: Option<&str>,
    ) -> (Artifact, String) {
        let fields: NewArtifact = serde_json::from_value(json!({
            "kind": "html",
            "title": "Page",
            "sessionId": session,
            "projectName": project
        }))
        .unwrap();
        let out = store.insert_artifact(&fields).unwrap();
        let token = out["token"].as_str().unwrap().to_owned();
        (
            serde_json::from_value(out["artifact"].clone()).unwrap(),
            token,
        )
    }

    fn comment(store: &Store, artifact_id: &str, anchor: Value) -> ArtifactComment {
        store
            .insert_artifact_comment(
                &serde_json::from_value(json!({
                    "artifactId": artifact_id,
                    "version": 1,
                    "anchor": anchor,
                    "body": "fix this"
                }))
                .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn inserts_reads_lists_and_renames_artifacts() {
        let store = test_support::store();
        let (a, token) = new_artifact(&store, Some("s1"), None);
        assert_eq!(token.len(), 32);
        assert!(!token.contains('-'));
        assert_eq!(store.get_artifact_token(&a.id).unwrap(), Some(token));
        assert_eq!(store.get_artifact_token("nope").unwrap(), None);
        assert_eq!(a.latest_version, 0.0);

        let json = serde_json::to_value(store.get_artifact(&a.id).unwrap().unwrap()).unwrap();
        assert_eq!(json["sessionId"], "s1");
        assert!(json.as_object().unwrap().contains_key("projectName"));
        assert!(json["projectName"].is_null());
        assert!(!json.as_object().unwrap().contains_key("gateRunId"));

        let (b, _) = new_artifact(&store, None, Some("p"));
        let (_c, _) = new_artifact(&store, None, None);
        let filter = ArtifactFilter {
            session_id: Some("s1".into()),
            project_name: Some("p".into()),
        };
        let ids: Vec<_> = store
            .list_artifacts(&filter, None)
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&a.id) && ids.contains(&b.id));
        assert_eq!(
            store
                .list_artifacts(&ArtifactFilter::default(), None)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            store
                .list_artifacts(&ArtifactFilter::default(), Some(1.0))
                .unwrap()
                .len(),
            1
        );

        store.rename_artifact(&a.id, "Renamed").unwrap();
        assert_eq!(store.get_artifact(&a.id).unwrap().unwrap().title, "Renamed");
        assert_eq!(store.list_artifact_ids().unwrap().len(), 3);
    }

    #[test]
    fn finds_the_gate_artifact() {
        let store = test_support::store();
        let fields: NewArtifact = serde_json::from_value(json!({
            "kind": "gate", "title": "Gate", "sessionId": null, "projectName": null,
            "gateRunId": "run", "gateNodeId": "node"
        }))
        .unwrap();
        store.insert_artifact(&fields).unwrap();
        let found = store.find_gate_artifact("run", "node").unwrap().unwrap();
        assert_eq!(found.gate_run_id.as_deref(), Some("run"));
        assert!(store.find_gate_artifact("run", "other").unwrap().is_none());
    }

    #[test]
    fn numbers_versions_in_order() {
        let mut store = test_support::store();
        let (a, _) = new_artifact(&store, None, None);
        let v1 = store.add_artifact_version(&a.id, "agent", None).unwrap();
        let v2 = store
            .add_artifact_version(&a.id, "user", Some("b1"))
            .unwrap();
        assert_eq!((v1.version, v2.version), (1.0, 2.0));
        let v1_json = serde_json::to_value(&v1).unwrap();
        assert!(!v1_json.as_object().unwrap().contains_key("answersBatchId"));
        assert_eq!(
            store.get_artifact(&a.id).unwrap().unwrap().latest_version,
            2.0
        );
        let listed = store.list_artifact_versions(&a.id).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].answers_batch_id.as_deref(), Some("b1"));
        assert_eq!(listed[1].author.0, "user");

        let err = store
            .add_artifact_version("missing", "agent", None)
            .unwrap_err();
        assert_eq!(err.to_string(), "Artifact not found: missing");
    }

    #[test]
    fn drafts_are_sent_as_a_batch_and_answered_by_a_version() {
        let mut store = test_support::store();
        let (a, _) = new_artifact(&store, None, None);
        store.add_artifact_version(&a.id, "agent", None).unwrap();
        assert!(store.send_artifact_drafts(&a.id).unwrap().is_none());

        let c1 = comment(&store, &a.id, json!({ "selector": "#x" }));
        let c2 = comment(&store, &a.id, json!(null));
        assert_eq!(c1.anchor, json!({ "selector": "#x" }));
        let c2_json = serde_json::to_value(&c2).unwrap();
        assert!(c2_json["anchor"].is_null());
        assert!(!c2_json.as_object().unwrap().contains_key("batchId"));
        assert!(!c2_json.as_object().unwrap().contains_key("sentAt"));

        let edited = store
            .update_artifact_comment(&c1.id, json!({ "body": "better" }).as_object().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            (edited.body.as_str(), &edited.anchor),
            ("better", &json!({ "selector": "#x" }))
        );
        let cleared = store
            .update_artifact_comment(
                &c1.id,
                json!({ "anchor": null, "body": null }).as_object().unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            (cleared.body.as_str(), &cleared.anchor),
            ("better", &Value::Null)
        );

        let drafts = ArtifactCommentFilter {
            state: Some(ArtifactCommentState("draft".into())),
            ..Default::default()
        };
        assert_eq!(
            store.list_artifact_comments(&a.id, &drafts).unwrap().len(),
            2
        );

        let sent = store.send_artifact_drafts(&a.id).unwrap().unwrap();
        let batch_id = sent["batchId"].as_str().unwrap().to_owned();
        assert_eq!(sent["comments"].as_array().unwrap().len(), 2);
        assert_eq!(sent["comments"][0]["state"], "sent");
        assert_eq!(sent["comments"][0]["batchId"], batch_id.as_str());
        assert!(store
            .list_artifact_comments(&a.id, &drafts)
            .unwrap()
            .is_empty());
        assert_eq!(
            store.unanswered_batch_id(&a.id).unwrap(),
            Some(batch_id.clone())
        );

        // A sent comment is part of the record.
        assert!(store
            .update_artifact_comment(&c1.id, json!({ "body": "late" }).as_object().unwrap())
            .unwrap()
            .is_none());
        assert!(!store.delete_artifact_comment(&c1.id).unwrap());
        assert_eq!(
            store.get_artifact_comment(&c1.id).unwrap().unwrap().body,
            "better"
        );

        store
            .add_artifact_version(&a.id, "agent", Some(&batch_id))
            .unwrap();
        assert_eq!(store.unanswered_batch_id(&a.id).unwrap(), None);

        let by_version = ArtifactCommentFilter {
            version: Some(1.0),
            ..Default::default()
        };
        assert_eq!(
            store
                .list_artifact_comments(&a.id, &by_version)
                .unwrap()
                .len(),
            2
        );
        let other = ArtifactCommentFilter {
            version: Some(2.0),
            ..Default::default()
        };
        assert!(store
            .list_artifact_comments(&a.id, &other)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn deletes_drafts_and_stale_artifacts() {
        let store = test_support::store();
        let (a, _) = new_artifact(&store, None, None);
        let c = comment(&store, &a.id, json!(""));
        assert!(c.anchor.is_null());
        assert!(store.delete_artifact_comment(&c.id).unwrap());
        assert!(store.get_artifact_comment(&c.id).unwrap().is_none());
        assert!(store
            .update_artifact_comment(&c.id, &Map::new())
            .unwrap()
            .is_none());

        assert!(store
            .delete_artifacts_updated_before("2000-01-01T00:00:00.000Z")
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .delete_artifacts_updated_before("9999-01-01T00:00:00.000Z")
                .unwrap(),
            [a.id]
        );
        assert!(store.list_artifact_ids().unwrap().is_empty());
    }
}

//! [`Store::call`]: a call by its TypeScript name, with its arguments as a
//! JSON array in signature order.
//!
//! The server sends every argument, `undefined` as `null`, so each call reads
//! a tuple of exactly its parameters. What comes back is what the TypeScript
//! function returns, `undefined` and `null` both as `null`.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use vorn_protocol::{
    ArtifactCommentFilter, ArtifactFilter, ConnectorInboxClaim, ConnectorInboxRetry,
    ConnectorPollError, ConnectorPollPage, NewArtifact, NewArtifactComment, NewDeviceToken,
    ProjectConfig, ScheduleLogEntry, SessionEvent, SessionGroupConfig, SourceConnection, SshKey,
    TaskConfig, TaskSourceLink, TerminalSession, WebhookEvent, WorkflowDefinition,
    WorkflowExecution, WorkspaceConfig,
};

use crate::{Error, Result, Store};

/// The arguments of `call`, as the tuple its signature has.
fn args<T: DeserializeOwned>(call: &str, args: Value) -> Result<T> {
    serde_json::from_value(args).map_err(|error| Error::BadArguments {
        call: call.to_owned(),
        error,
    })
}

fn out<T: Serialize>(value: T) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

/// A JS number argument used as a row id or count.
fn int(n: f64) -> i64 {
    n as i64
}

/// A number result, written as JavaScript writes it: `3`, not `3.0`.
fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.is_finite() && n.abs() <= 9_007_199_254_740_992.0 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

type Updates = Map<String, Value>;

pub(crate) fn call(store: &mut Store, call: &str, a: Value) -> Result<Value> {
    let c = call;
    match call {
        // Config.
        "seedSystemDefaults" => {
            crate::schema::seed_system_defaults(store)?;
            Ok(Value::Null)
        }
        "loadConfig" => store.load_config(),
        "saveConfig" => {
            let (config, deleted): (Value, Vec<String>) = args(c, a)?;
            store.save_config(&config, &deleted)?;
            Ok(Value::Null)
        }

        // Tasks.
        "dbListTasks" => {
            let (project, status): (Option<String>, Option<String>) = args(c, a)?;
            out(store.db_list_tasks(project.as_deref(), status.as_deref())?)
        }
        "dbGetTask" => {
            let (id,): (String,) = args(c, a)?;
            out(store.db_get_task(&id)?)
        }
        "dbInsertTask" => {
            let (task,): (TaskConfig,) = args(c, a)?;
            store.db_insert_task(&task)?;
            Ok(Value::Null)
        }
        "dbUpdateTask" => {
            let (id, updates, present): (String, Updates, Vec<String>) = args(c, a)?;
            store.db_update_task(&id, &updates, &present)?;
            Ok(Value::Null)
        }
        "dbDeleteTask" => {
            let (id,): (String,) = args(c, a)?;
            store.db_delete_task(&id)?;
            Ok(Value::Null)
        }
        "dbGetMaxTaskOrder" => {
            let (project,): (String,) = args(c, a)?;
            Ok(number(store.db_get_max_task_order(&project)?))
        }
        "dbFindTaskByConnectorExternalId" => {
            let (connector, external): (String, String) = args(c, a)?;
            out(store.db_find_task_by_connector_external_id(&connector, &external)?)
        }

        // Source connections.
        "dbListSourceConnections" => {
            let (connector,): (Option<String>,) = args(c, a)?;
            out(store.db_list_source_connections(connector.as_deref())?)
        }
        "dbGetSourceConnection" => {
            let (id,): (String,) = args(c, a)?;
            out(store.db_get_source_connection(&id)?)
        }
        "dbInsertSourceConnection" => {
            let (conn,): (SourceConnection,) = args(c, a)?;
            store.db_insert_source_connection(&conn)?;
            Ok(Value::Null)
        }
        "dbUpdateSourceConnection" => {
            let (id, updates, present): (String, Updates, Vec<String>) = args(c, a)?;
            store.db_update_source_connection(&id, &updates, &present)?;
            Ok(Value::Null)
        }
        "dbSetConnectionSignIn" => {
            let (id, signed_in_as, signed_in_at): (String, Option<String>, Option<String>) =
                args(c, a)?;
            store.db_set_connection_sign_in(
                &id,
                signed_in_as.as_deref(),
                signed_in_at.as_deref(),
            )?;
            Ok(Value::Null)
        }
        "dbDeleteSourceConnection" => {
            let (id,): (String,) = args(c, a)?;
            store.db_delete_source_connection(&id)?;
            Ok(Value::Null)
        }

        // Durable connector ingestion.
        "dbGetConnectorPollCursor" => {
            let (workflow, connection): (String, String) = args(c, a)?;
            out(store.db_get_connector_poll_cursor(&workflow, &connection)?)
        }
        "dbCountActiveConnectorInboxLeases" => {
            let (now,): (String,) = args(c, a)?;
            out(store.db_count_active_connector_inbox_leases(&now)?)
        }
        "dbEnqueueWebhookEvent" => {
            let (event,): (WebhookEvent,) = args(c, a)?;
            store.db_enqueue_webhook_event(&event)?;
            Ok(Value::Null)
        }
        "dbRecordConnectorPollPage" => {
            let (page,): (ConnectorPollPage,) = args(c, a)?;
            out(store.db_record_connector_poll_page(&page)?)
        }
        "dbRecordConnectorPollError" => {
            let (error,): (ConnectorPollError,) = args(c, a)?;
            store.db_record_connector_poll_error(&error)?;
            Ok(Value::Null)
        }
        "dbClaimConnectorInbox" => {
            let (claim,): (ConnectorInboxClaim,) = args(c, a)?;
            out(store.db_claim_connector_inbox(&claim)?)
        }
        "dbCompleteConnectorInbox" => {
            let (id, lease, processed_at): (f64, String, String) = args(c, a)?;
            out(store.db_complete_connector_inbox(int(id), &lease, &processed_at)?)
        }
        "dbRetryConnectorInbox" => {
            let (retry,): (ConnectorInboxRetry,) = args(c, a)?;
            out(store.db_retry_connector_inbox(&retry)?)
        }
        "dbDeferConnectorInbox" => {
            let (id, lease, available_at): (f64, String, String) = args(c, a)?;
            out(store.db_defer_connector_inbox(int(id), &lease, &available_at)?)
        }
        "dbRenewConnectorInboxLease" => {
            let (id, lease, lease_until): (f64, String, String) = args(c, a)?;
            out(store.db_renew_connector_inbox_lease(int(id), &lease, &lease_until)?)
        }
        "dbReleaseConnectorInboxLeases" => {
            let (now,): (String,) = args(c, a)?;
            store.db_release_connector_inbox_leases(&now)?;
            Ok(Value::Null)
        }

        // Task source links.
        "dbGetTaskSourceLink" => {
            let (task,): (String,) = args(c, a)?;
            out(store.db_get_task_source_link(&task)?)
        }
        "dbGetTaskSourceLinkByExternalId" => {
            let (connection, external): (String, String) = args(c, a)?;
            out(store.db_get_task_source_link_by_external_id(&connection, &external)?)
        }
        "dbListTaskSourceLinks" => {
            let (connection,): (String,) = args(c, a)?;
            out(store.db_list_task_source_links(&connection)?)
        }
        "dbInsertTaskSourceLink" => {
            let (link,): (TaskSourceLink,) = args(c, a)?;
            store.db_insert_task_source_link(&link)?;
            Ok(Value::Null)
        }
        "dbUpdateTaskSourceLink" => {
            let (task, updates): (String, Updates) = args(c, a)?;
            store.db_update_task_source_link(&task, &updates)?;
            Ok(Value::Null)
        }
        "dbDeleteTaskSourceLink" => {
            let (task,): (String,) = args(c, a)?;
            store.db_delete_task_source_link(&task)?;
            Ok(Value::Null)
        }

        // Projects.
        "dbListProjects" => out(store.db_list_projects()?),
        "dbGetProject" => {
            let (name,): (String,) = args(c, a)?;
            out(store.db_get_project(&name)?)
        }
        "dbInsertProject" => {
            let (project,): (ProjectConfig,) = args(c, a)?;
            store.db_insert_project(&project)?;
            Ok(Value::Null)
        }
        "dbUpdateProject" => {
            let (name, updates): (String, Updates) = args(c, a)?;
            store.db_update_project(&name, &updates)?;
            Ok(Value::Null)
        }
        "dbDeleteProject" => {
            let (name,): (String,) = args(c, a)?;
            store.db_delete_project(&name)?;
            Ok(Value::Null)
        }

        // Workflows.
        "dbListWorkflows" => out(store.db_list_workflows()?),
        "dbGetWorkflow" => {
            let (id,): (String,) = args(c, a)?;
            out(store.db_get_workflow(&id)?)
        }
        "dbInsertWorkflow" => {
            let (workflow,): (WorkflowDefinition,) = args(c, a)?;
            store.db_insert_workflow(&workflow)?;
            Ok(Value::Null)
        }
        "dbUpdateWorkflow" => {
            let (id, updates): (String, Updates) = args(c, a)?;
            out(store.db_update_workflow(&id, &updates)?)
        }
        "dbDeleteWorkflow" => {
            let (id,): (String,) = args(c, a)?;
            store.db_delete_workflow(&id)?;
            Ok(Value::Null)
        }
        "updateWorkflowRunStatus" => {
            let (id, at, status): (String, String, String) = args(c, a)?;
            store.update_workflow_run_status(&id, &at, &status)?;
            Ok(Value::Null)
        }

        // Identity and device tokens.
        "dbGetOwnerUser" => out(store.db_get_owner_user()?),
        "dbInsertDeviceToken" => {
            let (token,): (NewDeviceToken,) = args(c, a)?;
            store.db_insert_device_token(&token)?;
            Ok(Value::Null)
        }
        "dbGetDeviceTokenSecret" => {
            let (id,): (String,) = args(c, a)?;
            out(store.db_get_device_token_secret(&id)?)
        }
        "dbListDeviceTokens" => out(store.db_list_device_tokens()?),
        "dbHasDeviceTokens" => out(store.db_has_device_tokens()?),
        "dbRevokeDeviceToken" => {
            let (id, at): (String, String) = args(c, a)?;
            out(store.db_revoke_device_token(&id, &at)?)
        }
        "dbTouchDeviceToken" => {
            let (id, at): (String, String) = args(c, a)?;
            store.db_touch_device_token(&id, &at)?;
            Ok(Value::Null)
        }

        // Workspaces.
        "dbListWorkspaces" => out(store.db_list_workspaces()?),
        "dbInsertWorkspace" => {
            let (workspace,): (WorkspaceConfig,) = args(c, a)?;
            store.db_insert_workspace(&workspace)?;
            Ok(Value::Null)
        }
        "dbUpdateWorkspace" => {
            let (id, updates): (String, Updates) = args(c, a)?;
            store.db_update_workspace(&id, &updates)?;
            Ok(Value::Null)
        }
        "dbDeleteWorkspace" => {
            let (id,): (String,) = args(c, a)?;
            store.db_delete_workspace(&id)?;
            Ok(Value::Null)
        }

        // Session groups.
        "dbListSessionGroups" => out(store.db_list_session_groups()?),
        "dbInsertSessionGroup" => {
            let (group,): (SessionGroupConfig,) = args(c, a)?;
            store.db_insert_session_group(&group)?;
            Ok(Value::Null)
        }
        "dbUpdateSessionGroup" => {
            let (id, updates): (String, Updates) = args(c, a)?;
            store.db_update_session_group(&id, &updates)?;
            Ok(Value::Null)
        }
        "dbDeleteSessionGroup" => {
            let (id,): (String,) = args(c, a)?;
            store.db_delete_session_group(&id)?;
            Ok(Value::Null)
        }

        // SSH keys.
        "dbSaveSSHKey" => {
            let (key,): (SshKey,) = args(c, a)?;
            store.db_save_ssh_key(&key)?;
            Ok(Value::Null)
        }
        "dbListSSHKeys" => out(store.db_list_ssh_keys()?),
        "dbGetSSHKey" => {
            let (id,): (String,) = args(c, a)?;
            out(store.db_get_ssh_key(&id)?)
        }
        "dbDeleteSSHKey" => {
            let (id,): (String,) = args(c, a)?;
            store.db_delete_ssh_key(&id)?;
            Ok(Value::Null)
        }

        // Sessions.
        "saveSessions" => {
            let (sessions,): (Vec<TerminalSession>,) = args(c, a)?;
            store.save_sessions(&sessions)?;
            Ok(Value::Null)
        }
        "getPreviousSessions" => out(store.get_previous_sessions()?),
        "clearSessions" => {
            store.clear_sessions()?;
            Ok(Value::Null)
        }

        // Schedule log.
        "addScheduleLogEntry" => {
            let (entry,): (ScheduleLogEntry,) = args(c, a)?;
            store.add_schedule_log_entry(&entry)?;
            Ok(Value::Null)
        }
        "getScheduleLogEntries" => {
            let (workflow,): (Option<String>,) = args(c, a)?;
            out(store.get_schedule_log_entries(workflow.as_deref())?)
        }
        "clearScheduleLog" => {
            store.clear_schedule_log()?;
            Ok(Value::Null)
        }

        // Workflow runs.
        "saveWorkflowRun" => {
            let (execution,): (WorkflowExecution,) = args(c, a)?;
            out(store.save_workflow_run(&execution)?)
        }
        "listWorkflowRunIds" => out(store.list_workflow_run_ids()?),
        "dbGetWorkflowRunByConnectorInboxId" => {
            let (id,): (f64,) = args(c, a)?;
            out(store.db_get_workflow_run_by_connector_inbox_id(int(id))?)
        }
        "getWorkflowRun" => {
            let (id,): (String,) = args(c, a)?;
            out(store.get_workflow_run(&id)?)
        }
        "listWorkflowRuns" => {
            let (workflow, limit): (String, Option<f64>) = args(c, a)?;
            out(store.list_workflow_runs(&workflow, limit)?)
        }
        "listWorkflowRunsByTask" => {
            let (task, limit): (String, Option<f64>) = args(c, a)?;
            out(store.list_workflow_runs_by_task(&task, limit)?)
        }
        "listRunningRuns" => out(store.list_running_runs()?),
        "listRunsWithWaitingGates" => {
            let (kind,): (Option<String>,) = args(c, a)?;
            out(store.list_runs_with_waiting_gates(kind.as_deref())?)
        }
        "listAllWorkflowRuns" => {
            let (workspace, limit): (Option<String>, Option<f64>) = args(c, a)?;
            out(store.list_all_workflow_runs(workspace.as_deref(), limit)?)
        }

        // Effects.
        "claimEffect" => {
            let (effect, kind, now): (String, String, Option<f64>) = args(c, a)?;
            out(store.claim_effect(&effect, &kind, now)?)
        }
        "pruneEffectReceipts" => {
            let (kind, before): (String, f64) = args(c, a)?;
            out(store.prune_effect_receipts(&kind, before)?)
        }

        // Session events.
        "insertSessionEvent" => {
            let (event,): (SessionEvent,) = args(c, a)?;
            store.insert_session_event(&event)?;
            Ok(Value::Null)
        }
        "listSessionEvents" => {
            let (kind, limit): (Option<String>, Option<f64>) = args(c, a)?;
            out(store.list_session_events(kind.as_deref(), limit)?)
        }
        "listSessionEventsBySession" => {
            let (session, limit): (String, Option<f64>) = args(c, a)?;
            out(store.list_session_events_by_session(&session, limit)?)
        }

        // Artifacts.
        "insertArtifact" => {
            let (fields,): (NewArtifact,) = args(c, a)?;
            store.insert_artifact(&fields)
        }
        "getArtifact" => {
            let (id,): (String,) = args(c, a)?;
            out(store.get_artifact(&id)?)
        }
        "getArtifactToken" => {
            let (id,): (String,) = args(c, a)?;
            out(store.get_artifact_token(&id)?)
        }
        "listArtifacts" => {
            let (filter, limit): (Option<ArtifactFilter>, Option<f64>) = args(c, a)?;
            let filter = filter.unwrap_or_default();
            out(store.list_artifacts(&filter, limit)?)
        }
        "findGateArtifact" => {
            let (run, node): (String, String) = args(c, a)?;
            out(store.find_gate_artifact(&run, &node)?)
        }
        "renameArtifact" => {
            let (id, title): (String, String) = args(c, a)?;
            store.rename_artifact(&id, &title)?;
            Ok(Value::Null)
        }
        "addArtifactVersion" => {
            let (id, author, batch): (String, String, Option<String>) = args(c, a)?;
            out(store.add_artifact_version(&id, &author, batch.as_deref())?)
        }
        "listArtifactVersions" => {
            let (id,): (String,) = args(c, a)?;
            out(store.list_artifact_versions(&id)?)
        }
        "unansweredBatchId" => {
            let (id,): (String,) = args(c, a)?;
            out(store.unanswered_batch_id(&id)?)
        }
        "listArtifactComments" => {
            let (id, filter): (String, Option<ArtifactCommentFilter>) = args(c, a)?;
            let filter = filter.unwrap_or_default();
            out(store.list_artifact_comments(&id, &filter)?)
        }
        "getArtifactComment" => {
            let (id,): (String,) = args(c, a)?;
            out(store.get_artifact_comment(&id)?)
        }
        "insertArtifactComment" => {
            let (fields,): (NewArtifactComment,) = args(c, a)?;
            out(store.insert_artifact_comment(&fields)?)
        }
        "updateArtifactComment" => {
            let (id, change): (String, Updates) = args(c, a)?;
            out(store.update_artifact_comment(&id, &change)?)
        }
        "deleteArtifactComment" => {
            let (id,): (String,) = args(c, a)?;
            out(store.delete_artifact_comment(&id)?)
        }
        "sendArtifactDrafts" => {
            let (id,): (String,) = args(c, a)?;
            out(store.send_artifact_drafts(&id)?)
        }
        "deleteArtifactsUpdatedBefore" => {
            let (cutoff,): (String,) = args(c, a)?;
            out(store.delete_artifacts_updated_before(&cutoff)?)
        }
        "listArtifactIds" => out(store.list_artifact_ids()?),

        _ => Err(Error::UnknownCall(call.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use serde_json::json;

    #[test]
    fn answers_by_the_typescript_name() {
        let mut store = test_support::store();
        let task = json!({
            "id": "t1", "projectName": "p", "title": "T", "description": "",
            "status": "todo", "order": 3, "createdAt": "a", "updatedAt": "b"
        });
        store.call("dbInsertTask", json!([task])).unwrap();
        let listed = store.call("dbListTasks", json!(["p", null])).unwrap();
        assert_eq!(listed[0]["id"], "t1");
        assert_eq!(
            store.call("dbGetMaxTaskOrder", json!(["p"])).unwrap(),
            json!(3)
        );
        assert_eq!(
            store.call("dbGetTask", json!(["nope"])).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn names_the_call_and_the_field_a_bad_argument_is_in() {
        let mut store = test_support::store();
        let err = store
            .call("dbInsertTask", json!([{ "id": 1 }]))
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("dbInsertTask:"), "{err}");
        let err = store.call("dbDropEverything", json!([])).unwrap_err();
        assert!(matches!(err, Error::UnknownCall(_)));
    }
}

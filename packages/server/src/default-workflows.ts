import type {
  WorkflowDefinition,
  TaskStatusChangedTriggerConfig,
  LaunchAgentConfig,
  SourceConnection,
  ConnectorManifest,
  ConnectorPollTriggerConfig,
  CreateTaskFromItemConfig,
  TaskStatus
} from '@vornrun/shared/types'
import { connectorSeededWorkflowId } from '@vornrun/shared/types'

/** Stable id of the seeded "Default Task Workflow". */
export const DEFAULT_TASK_WORKFLOW_ID = 'system:default-task-workflow'

/**
 * Factory for the default task workflow seeded on first launch.
 *
 * Shape: a `taskStatusChanged` trigger (todo → in_progress, any project) wired
 * to a single headless `launchAgent` node whose `agentType` is `'fromTask'`.
 * At run time, `resolveEffectiveAgent` reads `task.assignedAgent` from the
 * trigger context, so the agent the user picks on each task is what actually
 * launches. The whole thing is editable in the workflow editor — users can
 * change the trigger, swap the agent, add steps, or disable/delete the
 * workflow outright. Nothing here is hidden or privileged; it's a worked
 * example that uses the same values any user could configure by hand.
 */
export function buildDefaultTaskWorkflow(): WorkflowDefinition {
  const triggerConfig: TaskStatusChangedTriggerConfig = {
    triggerType: 'taskStatusChanged',
    fromStatus: 'todo',
    toStatus: 'in_progress'
    // projectFilter omitted → fires in every project
  }

  const launchConfig: LaunchAgentConfig = {
    agentType: 'fromTask',
    projectName: '',
    projectPath: '',
    headless: true
  }

  return {
    id: DEFAULT_TASK_WORKFLOW_ID,
    name: 'Default Task Workflow',
    icon: 'Play',
    iconColor: '#10b981',
    enabled: true,
    workspaceId: 'personal',
    nodes: [
      {
        id: 'trigger-1',
        type: 'trigger',
        label: 'When task moves to In Progress',
        position: { x: 0, y: 0 },
        config: triggerConfig
      },
      {
        id: 'launch-1',
        type: 'launchAgent',
        label: 'Launch task agent',
        position: { x: 0, y: 120 },
        config: launchConfig
      }
    ],
    edges: [{ id: 'e1', source: 'trigger-1', target: 'launch-1' }]
  }
}

/**
 * A cron expression that fires every `minutes`.
 *
 * The minute field only counts to 59, so a step of 60 in it is not "hourly" —
 * it is invalid, and a connector suggesting an hour would have been seeded with
 * a schedule that never fires. An interval of an hour or more moves to the hour
 * field instead; one that does not divide evenly into hours is rounded to the
 * nearest hour rather than kept as an unschedulable minute step.
 */
export function cronEveryMinutes(minutes: number): string {
  if (minutes <= 1) return '* * * * *'
  if (minutes < 60) return `*/${minutes} * * * *`
  const hours = Math.round(minutes / 60)
  if (hours <= 1) return '0 * * * *'
  // A day is its own expression, not a 24-step in a field that counts 0-23.
  // Capping to 23 instead would fire every 23 hours and drift a full hour
  // earlier each day, which nobody asking for "daily" wants.
  if (hours >= 24) return '0 0 * * *'
  return `0 */${hours} * * *`
}

/**
 * Build a seeded workflow for a (connection × manifest event). The graph is
 * `[connectorPoll trigger] → [createTaskFromItem node]`. Fully visible and
 * editable in the workflow editor — users can add condition/launchAgent nodes
 * downstream, change the cron, or disable/delete it. The stable id means the
 * workflow is tied to this connection: deleting the connection removes its
 * seeded workflows, and deleting the workflow sticks because no background
 * process re-seeds (seeding only happens on connection:create).
 */
export function buildConnectorSeededWorkflow(
  connection: SourceConnection,
  manifest: ConnectorManifest,
  event: NonNullable<ConnectorManifest['defaultWorkflows']>[number]
): WorkflowDefinition {
  const id = connectorSeededWorkflowId(connection.id, event.event)
  const cron = cronEveryMinutes(Math.max(1, Math.round(event.defaultCronFromMinutes)))

  const triggerConfig: ConnectorPollTriggerConfig = {
    triggerType: 'connectorPoll',
    connectionId: connection.id,
    event: event.event,
    cron
  }

  // Pick a sensible initial status from the connector's statusMapping (if any),
  // else default to 'todo'. The user can change this in the node's config form.
  const initialStatus: TaskStatus =
    (manifest.statusMapping && manifest.statusMapping[0]?.suggestedLocal) || 'todo'

  const nodeConfig: CreateTaskFromItemConfig = {
    nodeType: 'createTaskFromItem',
    project: 'fromConnection',
    initialStatus
  }

  return {
    id,
    name: event.name,
    icon: connection.connectorId,
    iconColor: '#64748b',
    enabled: true,
    workspaceId: 'personal',
    nodes: [
      {
        id: 'trigger-1',
        type: 'trigger',
        label: `Poll ${manifest.triggers?.find((t) => t.type === event.event)?.label || event.event}`,
        position: { x: 0, y: 0 },
        config: triggerConfig
      },
      {
        id: 'create-1',
        type: 'createTaskFromItem',
        label: 'Create task from item',
        position: { x: 0, y: 120 },
        config: nodeConfig
      }
    ],
    edges: [{ id: 'e1', source: 'trigger-1', target: 'create-1' }]
  }
}

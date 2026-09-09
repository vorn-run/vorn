import type {
  ConnectorItemContext,
  WorkflowDefinition,
  WorkflowExecution
} from '@vornrun/shared/types'
import { schedulerExecutionContext } from '@vornrun/shared/workflow-graph'
import { configManager } from '../config-manager'
import log from '../logger'
import {
  adoptConnectorInboxLease,
  executeWorkflow,
  reconcileRunningExecutions,
  rescheduleWaitingGateTimers
} from './engine'
import { api } from './host'

/**
 * What the scheduler does when a trigger fires.
 *
 * This is the handler a window used to run on `scheduler:execute`, moved to the
 * side of the wire the run now happens on. It keeps the same shape, including
 * the two things easy to lose: a workflow that has since been deleted still has
 * to release the inbox row it was delivered with, and a run that was already
 * going gets its lease adopted rather than being started a second time.
 */
function workflowById(id: string): WorkflowDefinition | undefined {
  return configManager.loadConfig().workflows?.find((w) => w.id === id)
}

export async function runScheduled(
  workflowId: string,
  inputs?: Record<string, unknown>
): Promise<void> {
  const workflow = workflowById(workflowId)
  if (!workflow) return

  try {
    await executeWorkflow(workflow, schedulerExecutionContext(undefined, inputs), {
      source: 'scheduler'
    })
  } catch (err) {
    log.warn({ err, workflowId }, '[scheduler] a scheduled workflow did not complete')
  }
}

export async function runConnectorItem(event: {
  workflowId: string
  connectorItem: ConnectorItemContext
  connectorInboxId?: number
  connectorInboxLeaseToken?: string
  existingExecution?: WorkflowExecution
}): Promise<void> {
  const { workflowId, connectorItem, connectorInboxId, connectorInboxLeaseToken } = event
  const workflow = workflowById(workflowId)

  if (!workflow) {
    // The row outlived its workflow. Defer rather than drop, so the item is
    // still there if the workflow comes back.
    if (connectorInboxId !== undefined && connectorInboxLeaseToken) {
      await api.completeConnectorInbox({
        id: connectorInboxId,
        leaseToken: connectorInboxLeaseToken,
        disposition: 'defer'
      })
    }
    return
  }

  if (event.existingExecution) {
    await adoptConnectorInboxLease(event.existingExecution, connectorItem)
    rescheduleWaitingGateTimers([event.existingExecution], [workflow])
    await reconcileRunningExecutions([event.existingExecution], [workflow])
    return
  }

  try {
    const execution = await executeWorkflow(
      workflow,
      schedulerExecutionContext(connectorItem, undefined),
      { source: 'scheduler' }
    )
    // The row may have been re-leased while the run was starting.
    if (
      connectorInboxLeaseToken &&
      execution.connectorInboxLeaseToken !== connectorInboxLeaseToken
    ) {
      await adoptConnectorInboxLease(execution, connectorItem)
    }
  } catch (err) {
    log.warn({ err, workflowId }, '[connector] a delivered item did not complete')
  }
}

import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { ConnectorItemContext, WorkflowDefinition } from '../packages/shared/src/types'

const workflows: WorkflowDefinition[] = []
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: { loadConfig: () => ({ workflows }) }
}))

const executeWorkflow = vi.hoisted(() => vi.fn())
const adoptConnectorInboxLease = vi.hoisted(() => vi.fn(async () => {}))
const reconcileRunningExecutions = vi.hoisted(() => vi.fn(async () => {}))
const rescheduleWaitingGateTimers = vi.hoisted(() => vi.fn())
vi.mock('../packages/server/src/workflows/engine', () => ({
  executeWorkflow,
  adoptConnectorInboxLease,
  reconcileRunningExecutions,
  rescheduleWaitingGateTimers
}))

const completeConnectorInbox = vi.hoisted(() => vi.fn(async () => {}))
vi.mock('../packages/server/src/workflows/host', () => ({ api: { completeConnectorInbox } }))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { runConnectorItem, runScheduled } from '../packages/server/src/workflows/dispatch'

/**
 * What the scheduler does when a trigger fires.
 *
 * This was a handler in a window, which is why a scheduled run with nothing
 * open went nowhere. The two things worth pinning are the ones easy to lose in
 * the move: a workflow that has since been deleted still has to release the
 * inbox row it was delivered with, and a run already going gets its lease
 * adopted rather than being started a second time.
 */
const workflow = (id = 'wf-1'): WorkflowDefinition =>
  ({ id, name: 'Nightly', enabled: true, nodes: [], edges: [] }) as unknown as WorkflowDefinition

const item = (): ConnectorItemContext =>
  ({
    connectionId: 'conn-1',
    connectorId: 'github',
    externalId: '7',
    raw: {}
  }) as ConnectorItemContext

beforeEach(() => {
  workflows.length = 0
  executeWorkflow
    .mockReset()
    .mockResolvedValue({ runId: 'run-1', connectorInboxLeaseToken: 'lease' })
  adoptConnectorInboxLease.mockClear()
  reconcileRunningExecutions.mockClear()
  rescheduleWaitingGateTimers.mockClear()
  completeConnectorInbox.mockClear()
})

describe('a schedule firing', () => {
  it('runs the workflow it names', async () => {
    workflows.push(workflow())
    await runScheduled('wf-1', { which: 'first' })

    expect(executeWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'wf-1' }),
      { connectorItem: undefined, inputs: { which: 'first' } },
      { source: 'scheduler' }
    )
  })

  it('does nothing for a workflow that has since been deleted', async () => {
    await runScheduled('wf-gone')
    expect(executeWorkflow).not.toHaveBeenCalled()
  })

  it('survives a run that throws, because the next tick still has to happen', async () => {
    workflows.push(workflow())
    executeWorkflow.mockRejectedValue(new Error('nope'))

    await expect(runScheduled('wf-1')).resolves.toBeUndefined()
  })
})

describe('a connector item arriving', () => {
  it('runs the workflow with the item as its context', async () => {
    workflows.push(workflow())
    await runConnectorItem({
      workflowId: 'wf-1',
      connectorItem: item(),
      connectorInboxId: 11,
      connectorInboxLeaseToken: 'lease'
    })

    expect(executeWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'wf-1' }),
      expect.objectContaining({ connectorItem: expect.objectContaining({ externalId: '7' }) }),
      { source: 'scheduler' }
    )
  })

  it('defers the row when its workflow is gone, rather than dropping it', async () => {
    await runConnectorItem({
      workflowId: 'wf-gone',
      connectorItem: item(),
      connectorInboxId: 11,
      connectorInboxLeaseToken: 'lease'
    })

    expect(completeConnectorInbox).toHaveBeenCalledWith({
      id: 11,
      leaseToken: 'lease',
      disposition: 'defer'
    })
    expect(executeWorkflow).not.toHaveBeenCalled()
  })

  it('adopts the lease of a run that is already going instead of starting another', async () => {
    workflows.push(workflow())
    const existingExecution = { runId: 'run-existing' } as never

    await runConnectorItem({
      workflowId: 'wf-1',
      connectorItem: item(),
      connectorInboxId: 11,
      connectorInboxLeaseToken: 'lease',
      existingExecution
    })

    expect(adoptConnectorInboxLease).toHaveBeenCalledWith(existingExecution, expect.anything())
    expect(rescheduleWaitingGateTimers).toHaveBeenCalled()
    expect(reconcileRunningExecutions).toHaveBeenCalled()
    expect(executeWorkflow).not.toHaveBeenCalled()
  })

  it('re-adopts when the row was leased again while the run was starting', async () => {
    workflows.push(workflow())
    executeWorkflow.mockResolvedValue({ runId: 'run-1', connectorInboxLeaseToken: 'a-newer-lease' })

    await runConnectorItem({
      workflowId: 'wf-1',
      connectorItem: item(),
      connectorInboxId: 11,
      connectorInboxLeaseToken: 'lease'
    })

    expect(adoptConnectorInboxLease).toHaveBeenCalled()
  })
})

// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { WorkflowExecution } from '../src/shared/types'

const sendWorkflowGateNotification = vi.hoisted(() => vi.fn())
vi.mock('../src/renderer/lib/notifications', () => ({ sendWorkflowGateNotification }))

const setEditingWorkflowId = vi.fn()
const setWorkflowEditorOpen = vi.fn()
const mockState = {
  config: {
    workflows: [
      {
        id: 'wf-1',
        name: 'Nightly review',
        nodes: [{ id: 'gate', type: 'approval', label: 'Ship it?', config: { message: 'ok?' } }]
      }
    ]
  },
  setEditingWorkflowId,
  setWorkflowEditorOpen
}
vi.mock('../src/renderer/stores', () => ({
  useAppStore: { getState: () => mockState }
}))

import { announceRun, resetAnnouncedRuns } from '../src/renderer/lib/run-notifications'

/**
 * The window's half of a run happening in the server.
 *
 * Every update carries the whole run, so the thing that must not slip is
 * announcing a *transition*: without that, one gate would be announced again on
 * every step that followed it.
 */
const run = (over: Partial<WorkflowExecution> = {}): WorkflowExecution =>
  ({
    runId: 'run-1',
    workflowId: 'wf-1',
    startedAt: new Date().toISOString(),
    status: 'running',
    nodeStates: [{ nodeId: 'gate', status: 'pending' }],
    ...over
  }) as WorkflowExecution

const waiting = (): WorkflowExecution =>
  run({ nodeStates: [{ nodeId: 'gate', status: 'waiting' }] })

beforeEach(() => {
  resetAnnouncedRuns()
  sendWorkflowGateNotification.mockClear()
  setEditingWorkflowId.mockClear()
  ;(globalThis as unknown as { Notification: unknown }).Notification = { permission: 'denied' }
})

describe('a gate that starts waiting', () => {
  it('is announced once, however many updates carry it', () => {
    announceRun(waiting())
    announceRun(waiting())
    announceRun(waiting())

    expect(sendWorkflowGateNotification).toHaveBeenCalledTimes(1)
    expect(sendWorkflowGateNotification).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'wf-1' }),
      'gate',
      'Ship it?',
      'ok?',
      mockState.config,
      expect.any(Function)
    )
  })

  it('opens the editor on the workflow when the notification is clicked', () => {
    announceRun(waiting())

    const onClick = sendWorkflowGateNotification.mock.calls[0][5] as () => void
    onClick()

    expect(setEditingWorkflowId).toHaveBeenCalledWith('wf-1')
    expect(setWorkflowEditorOpen).toHaveBeenCalledWith(true)
  })

  it('says nothing about a workflow this window does not know', () => {
    announceRun(
      run({ workflowId: 'wf-unknown', nodeStates: [{ nodeId: 'gate', status: 'waiting' }] })
    )

    expect(sendWorkflowGateNotification).not.toHaveBeenCalled()
  })
})

describe('a run that ends', () => {
  it('is announced when it was seen running first', () => {
    const notify = vi.fn()
    ;(globalThis as unknown as { Notification: unknown }).Notification = Object.assign(notify, {
      permission: 'granted'
    })

    announceRun(run())
    announceRun(run({ status: 'success', nodeStates: [{ nodeId: 'gate', status: 'success' }] }))

    expect(notify).toHaveBeenCalledWith('Vorn', {
      body: expect.stringContaining('Nightly review')
    })
  })

  it('is not announced when the window only ever saw it finished', () => {
    const notify = vi.fn()
    ;(globalThis as unknown as { Notification: unknown }).Notification = Object.assign(notify, {
      permission: 'granted'
    })

    announceRun(run({ status: 'error' }))

    expect(notify).not.toHaveBeenCalled()
  })

  it('forgets it, so a later run of the same id starts clean', () => {
    announceRun(waiting())
    announceRun(run({ status: 'success' }))
    announceRun(waiting())

    expect(sendWorkflowGateNotification).toHaveBeenCalledTimes(2)
  })

  it('forgets a stopped one too, which raises nothing to say it is over', () => {
    announceRun(waiting())
    announceRun(run({ status: 'cancelled' }))
    announceRun(waiting())

    expect(sendWorkflowGateNotification).toHaveBeenCalledTimes(2)
  })
})

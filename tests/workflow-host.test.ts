import { describe, it, expect, vi, beforeEach } from 'vitest'
import { IPC } from '../packages/shared/src/types'

const broadcast = vi.hoisted(() => vi.fn())
vi.mock('../packages/server/src/broadcast', () => ({ clientRegistry: { broadcast } }))

const callMethod = vi.hoisted(() => vi.fn(async (_method: string, _params?: unknown) => undefined))
vi.mock('../packages/server/src/ws-handler', () => ({ callMethod }))

const headlessManager = vi.hoisted(() => {
  const listeners = new Map<string, Set<(...a: unknown[]) => void>>()
  return {
    on: (event: string, fn: (...a: unknown[]) => void) => {
      const set = listeners.get(event) ?? new Set()
      set.add(fn)
      listeners.set(event, set)
    },
    off: (event: string, fn: (...a: unknown[]) => void) => listeners.get(event)?.delete(fn),
    emit: (event: string, ...args: unknown[]) => listeners.get(event)?.forEach((fn) => fn(...args)),
    removeAllListeners: () => listeners.clear()
  }
})
vi.mock('../packages/server/src/headless-manager', () => ({ headlessManager }))

const scriptRunnerEvents = vi.hoisted(() => {
  const listeners = new Map<string, Set<(...a: unknown[]) => void>>()
  return {
    on: (event: string, fn: (...a: unknown[]) => void) => {
      const set = listeners.get(event) ?? new Set()
      set.add(fn)
      listeners.set(event, set)
    },
    off: (event: string, fn: (...a: unknown[]) => void) => listeners.get(event)?.delete(fn),
    emit: (event: string, ...args: unknown[]) => listeners.get(event)?.forEach((fn) => fn(...args)),
    removeAllListeners: () => listeners.clear()
  }
})
vi.mock('../packages/server/src/script-runner', () => ({ scriptRunnerEvents }))

vi.mock('../packages/server/src/pty-manager', () => ({
  ptyManager: { getActiveSessions: () => [] }
}))
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: () => ({
      tasks: [
        { id: 'a', projectName: 'vorn', status: 'todo', order: 2 },
        { id: 'b', projectName: 'vorn', status: 'todo', order: 1 },
        { id: 'c', projectName: 'other', status: 'todo', order: 0 }
      ]
    })
  }
}))
vi.mock('../packages/server/src/database', () => ({ getWorkflowRun: () => null }))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  api,
  nextTask,
  onHeadlessData,
  onHeadlessExit,
  onScriptData
} from '../packages/server/src/workflows/host'

/**
 * The seam the engine reaches the rest of the server through.
 *
 * It exists because the engine used to be in a window: every call it makes is
 * a call the app made a moment ago, and keeping the shape means the engine did
 * not have to be rewritten to move. What is worth pinning is that the calls go
 * somewhere real, and that a run going out on the wire is not forgotten.
 */
beforeEach(() => {
  broadcast.mockClear()
  callMethod.mockClear()
  headlessManager.removeAllListeners()
  scriptRunnerEvents.removeAllListeners()
})

describe('what the engine asks the server for', () => {
  it('starts a session the way a person does, through the method a client calls', async () => {
    await api.createHeadlessSession({ agentType: 'claude' } as never)
    expect(callMethod).toHaveBeenCalledWith('headless:create', { agentType: 'claude' })
  })

  it('routes every call it offers through a registered method', async () => {
    const calls: [string, unknown][] = [
      [
        'connector:inboxComplete',
        await api.completeConnectorInbox({ id: 1, leaseToken: 't', disposition: 'processed' })
      ],
      ['connector:inboxRenew', await api.renewConnectorInbox({ id: 1, leaseToken: 't' })],
      ['sessionEvent:listBySession', await api.listSessionEventsBySession('s1', 5)],
      ['script:execute', await api.executeScript({ scriptContent: 'x' } as never)],
      [
        'connection:executeAction',
        await api.executeConnectorAction({ connectionId: 'c', action: 'a', args: {} })
      ],
      ['http:request', await api.httpRequest({ method: 'GET', url: 'https://x' })],
      [
        'connection:upsertFromItem',
        await api.upsertTaskFromItem({
          connectionId: 'c',
          item: {} as never,
          initialStatus: 'todo'
        })
      ],
      ['headless:kill', await api.killHeadlessSession('s1')],
      ['terminal:create', await api.createTerminal({ agentType: 'claude' } as never)],
      ['workflow:runManual', await api.runWorkflowManual('wf-1', { a: 1 })],
      ['workflowRun:claim', await api.claimWorkflowRun({ workflowId: 'wf-1' })],
      ['workflowRun:release', await api.releaseWorkflowRun({ workflowId: 'wf-1', runId: 'r' })],
      [
        'workflow:executionComplete',
        await api.reportWorkflowComplete({
          workflowId: 'wf-1',
          workflowName: 'W',
          completedAt: 'now',
          status: 'success',
          sessionsLaunched: 1
        })
      ],
      ['worktree:activeSessions', await api.getWorktreeActiveSessions('/w')],
      ['git:worktreeDirty', await api.isWorktreeDirty('/w')],
      ['git:removeWorktree', await api.removeWorktree('/p', '/w', false)]
    ]

    expect(calls).toHaveLength(16)
    expect(callMethod.mock.calls.map((call) => call[0])).toEqual(calls.map((call) => call[0]))
  })

  it('saves a run and tells whoever is watching, in that order', async () => {
    const execution = { runId: 'run-1', workflowId: 'wf-1' }
    await api.saveWorkflowRun(execution as never)

    expect(callMethod).toHaveBeenCalledWith('workflowRun:save', execution)
    expect(broadcast).toHaveBeenCalledWith(IPC.WORKFLOW_RUN_UPDATED, execution)
    expect(callMethod.mock.invocationCallOrder[0]).toBeLessThan(
      broadcast.mock.invocationCallOrder[0]
    )
  })
})

describe('the streams a step listens to', () => {
  it('hears headless output and exits, and stops when it lets go', () => {
    const data = vi.fn()
    const exit = vi.fn()
    const stopData = onHeadlessData(data)
    const stopExit = onHeadlessExit(exit)

    headlessManager.emit('client-message', IPC.HEADLESS_DATA, { id: 's1', data: 'hello' })
    headlessManager.emit('client-message', IPC.HEADLESS_EXIT, { id: 's1', exitCode: 0 })
    expect(data).toHaveBeenCalledWith({ id: 's1', data: 'hello' })
    expect(exit).toHaveBeenCalledWith({ id: 's1', exitCode: 0 })

    stopData()
    stopExit()
    headlessManager.emit('client-message', IPC.HEADLESS_DATA, { id: 's1', data: 'more' })
    expect(data).toHaveBeenCalledTimes(1)
  })

  it('ignores messages from the same emitter that are not its own', () => {
    const data = vi.fn()
    onHeadlessData(data)

    headlessManager.emit('client-message', IPC.SESSION_UPDATED, { id: 's1' })
    expect(data).not.toHaveBeenCalled()
  })

  it('hears script output by run', () => {
    const data = vi.fn()
    const stop = onScriptData(data)

    scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId: 'r1', data: 'out' })
    expect(data).toHaveBeenCalledWith({ runId: 'r1', data: 'out' })

    stop()
    scriptRunnerEvents.emit(IPC.SCRIPT_DATA, { runId: 'r1', data: 'more' })
    expect(data).toHaveBeenCalledTimes(1)
  })
})

describe('the task a step is pointed at', () => {
  it('is the first in the project queue, by order', () => {
    expect(nextTask('vorn')?.id).toBe('b')
  })

  it('is nothing when the project has none', () => {
    expect(nextTask('empty')).toBeUndefined()
  })
})

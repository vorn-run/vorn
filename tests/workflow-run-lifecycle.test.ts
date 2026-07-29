// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { WorkflowDefinition, WorkflowExecution } from '../src/shared/types'

/**
 * Covers what used to wedge a run permanently: a headless step whose exit event
 * never arrives, or arrives before the step knows its own session id. Both left
 * the step awaiting forever, and because the engine held a per-workflow lock for
 * the duration, every later run of that workflow was dropped too.
 */

type ExitListener = (p: { id: string; exitCode: number }) => void

const exitListeners = new Set<ExitListener>()

function emitExit(id: string, exitCode: number): void {
  for (const l of [...exitListeners]) l({ id, exitCode })
}

const claims = new Map<string, string>()
let runSeq = 0
let sessionSeq = 0

/** Stands in for the core registry, with the same grant/release semantics. */
const claimWorkflowRun = vi.fn(
  ({ workflowId, params }: { workflowId: string; params?: string }) => {
    const key = JSON.stringify([workflowId, params || 'manual'])
    const held = claims.get(key)
    if (held) return Promise.resolve({ granted: false, runId: held })
    const runId = `run-${++runSeq}`
    claims.set(key, runId)
    return Promise.resolve({ granted: true, runId })
  }
)

const releaseWorkflowRun = vi.fn(
  ({ workflowId, params, runId }: { workflowId: string; params?: string; runId: string }) => {
    const key = JSON.stringify([workflowId, params || 'manual'])
    if (claims.get(key) === runId) claims.delete(key)
    return Promise.resolve()
  }
)

const killHeadlessSession = vi.fn(() => Promise.resolve())

/** Resolves once the given session has been created, so tests can await launch. */
let onSessionCreated: ((id: string) => void) | null = null

const createHeadlessSession = vi.fn(() => {
  const id = `sess-${++sessionSeq}`
  queueMicrotask(() => onSessionCreated?.(id))
  return Promise.resolve({ id, agentSessionId: undefined, worktreePath: undefined })
})

const mockState = {
  config: {
    defaults: { defaultAgent: 'claude', headlessStepTimeoutMinutes: 60 },
    projects: [{ name: 'p', path: '/p' }],
    tasks: [],
    workflows: [] as WorkflowDefinition[]
  },
  workflowExecutions: new Map<string, WorkflowExecution>(),
  terminals: new Map(),
  headlessSessions: [] as { id: string; agentSessionId?: string }[],
  setWorkflowExecution: (runId: string, execution: WorkflowExecution) => {
    mockState.workflowExecutions.set(runId, execution)
  },
  addHeadlessSession: vi.fn(),
  startTask: vi.fn(),
  reopenTask: vi.fn(),
  getNextTask: vi.fn(),
  setEditingWorkflowId: vi.fn(),
  setWorkflowEditorOpen: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: { getState: () => mockState }
}))

vi.mock('../src/renderer/lib/notifications', () => ({
  sendWorkflowGateNotification: vi.fn()
}))

const { executeWorkflow, stopWorkflowRun } = await import('../src/renderer/lib/workflow-execution')

function makeWorkflow(id = 'wf-1'): WorkflowDefinition {
  return {
    id,
    name: 'Test Workflow',
    icon: 'Rocket',
    enabled: true,
    nodes: [
      { id: 'trigger', type: 'trigger', label: 'Trigger', position: { x: 0, y: 0 }, config: {} },
      {
        id: 'agent',
        type: 'launchAgent',
        label: 'Agent',
        position: { x: 0, y: 1 },
        config: {
          agentType: 'claude',
          projectName: 'p',
          projectPath: '/p',
          headless: true,
          prompt: 'do the thing'
        }
      }
    ],
    edges: [{ id: 'e1', source: 'trigger', target: 'agent' }]
  } as unknown as WorkflowDefinition
}

/** Waits for the launched session id, so a test can drive its exit. */
function nextSession(): Promise<string> {
  return new Promise((resolve) => {
    onSessionCreated = (id) => {
      onSessionCreated = null
      resolve(id)
    }
  })
}

beforeEach(() => {
  vi.useFakeTimers()
  // The engine posts a desktop notification on completion; jsdom has no
  // Notification constructor.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  ;(globalThis as any).Notification = { permission: 'denied' }
  exitListeners.clear()
  claims.clear()
  runSeq = 0
  sessionSeq = 0
  onSessionCreated = null
  mockState.workflowExecutions.clear()
  mockState.config.workflows = []
  mockState.config.defaults.headlessStepTimeoutMinutes = 60
  claimWorkflowRun.mockClear()
  releaseWorkflowRun.mockClear()
  killHeadlessSession.mockClear()
  createHeadlessSession.mockClear()

  globalThis.window.api = {
    claimWorkflowRun,
    releaseWorkflowRun,
    createHeadlessSession,
    killHeadlessSession,
    saveWorkflowRun: vi.fn(() => Promise.resolve()),
    reportWorkflowComplete: vi.fn(() => Promise.resolve()),
    runWorkflowManual: vi.fn(() => Promise.resolve()),
    listSessionEventsBySession: vi.fn(() => Promise.resolve([])),
    getWorktreeActiveSessions: vi.fn(() => Promise.resolve({ count: 0 })),
    isWorktreeDirty: vi.fn(() => Promise.resolve(false)),
    removeWorktree: vi.fn(() => Promise.resolve()),
    onHeadlessData: () => () => {},
    onHeadlessExit: (fn: ExitListener) => {
      exitListeners.add(fn)
      return () => exitListeners.delete(fn)
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
  } as any
})

afterEach(() => {
  vi.useRealTimers()
})

describe('headless step completion', () => {
  it('completes the run when the agent exits cleanly', async () => {
    const wf = makeWorkflow()
    const runPromise = executeWorkflow(wf)

    const sessionId = await nextSession()
    emitExit(sessionId, 0)

    const execution = await runPromise
    expect(execution.status).toBe('success')
    expect(execution.nodeStates.find((n) => n.nodeId === 'agent')?.status).toBe('success')
  })

  it('does not lose an exit that arrives before the session id is known', async () => {
    // The Windows headless path spawns through cmd.exe, so a failing shim can
    // exit before createHeadlessSession's reply reaches the renderer. That exit
    // used to be dropped, leaving the step awaiting forever.
    const wf = makeWorkflow()
    onSessionCreated = (id) => {
      onSessionCreated = null
      emitExit(id, 1)
    }

    const execution = await executeWorkflow(wf)

    expect(execution.status).toBe('error')
    const agent = execution.nodeStates.find((n) => n.nodeId === 'agent')
    expect(agent?.status).toBe('error')
    expect(agent?.error).toBe('Exit code 1')
  })

  it('kills the agent and fails the step when no exit ever arrives', async () => {
    const wf = makeWorkflow()
    mockState.config.defaults.headlessStepTimeoutMinutes = 1
    const runPromise = executeWorkflow(wf)

    const sessionId = await nextSession()
    await vi.advanceTimersByTimeAsync(60_000 + 10)

    const execution = await runPromise
    expect(killHeadlessSession).toHaveBeenCalledWith(sessionId)
    expect(execution.status).toBe('error')
    expect(execution.nodeStates.find((n) => n.nodeId === 'agent')?.error).toMatch(/timed out/i)
  })

  it('releases the trigger claim after a timed-out run, so the next one is not blocked', async () => {
    const wf = makeWorkflow()
    mockState.config.defaults.headlessStepTimeoutMinutes = 1
    const runPromise = executeWorkflow(wf)

    await nextSession()
    await vi.advanceTimersByTimeAsync(60_000 + 10)
    await runPromise

    expect(releaseWorkflowRun).toHaveBeenCalled()
    // The claim being free is what the old per-workflow lock never allowed.
    const second = executeWorkflow(wf)
    const sessionId = await nextSession()
    emitExit(sessionId, 0)
    expect((await second).status).toBe('success')
  })
})

describe('run concurrency', () => {
  it('runs the same workflow in parallel for different trigger parameters', async () => {
    const wf = makeWorkflow()
    const ids: string[] = []
    onSessionCreated = (id) => ids.push(id)

    const runA = executeWorkflow(wf, {
      connectorItem: { connectionId: 'c1', externalId: 'issue-7', title: 'A', raw: {} }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any)
    const runB = executeWorkflow(wf, {
      connectorItem: { connectionId: 'c1', externalId: 'issue-8', title: 'B', raw: {} }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any)

    await vi.advanceTimersByTimeAsync(0)
    expect(ids).toHaveLength(2)

    ids.forEach((id) => emitExit(id, 0))
    const [a, b] = await Promise.all([runA, runB])

    expect(a.runId).not.toBe(b.runId)
    expect(a.status).toBe('success')
    expect(b.status).toBe('success')
  })

  it('collapses two identical triggers fired at once into a single run', async () => {
    const wf = makeWorkflow()
    const ids: string[] = []
    onSessionCreated = (id) => ids.push(id)

    const first = executeWorkflow(wf)
    await vi.advanceTimersByTimeAsync(0)
    // Second instance hearing the same tick.
    const second = executeWorkflow(wf)
    await vi.advanceTimersByTimeAsync(0)

    expect(ids).toHaveLength(1)
    ids.forEach((id) => emitExit(id, 0))

    const [a, b] = await Promise.all([first, second])
    expect(b.runId).toBe(a.runId)
    expect(createHeadlessSession).toHaveBeenCalledTimes(1)
  })
})

describe('stopping a run', () => {
  it('kills the run’s sessions and closes it as cancelled', async () => {
    const wf = makeWorkflow()
    mockState.config.workflows = [wf]
    const runPromise = executeWorkflow(wf)

    const sessionId = await nextSession()
    await vi.advanceTimersByTimeAsync(0)

    const runId = [...mockState.workflowExecutions.keys()][0]
    await stopWorkflowRun(runId)

    const execution = await runPromise
    expect(killHeadlessSession).toHaveBeenCalledWith(sessionId)
    expect(execution.status).toBe('cancelled')
    expect(execution.nodeStates.find((n) => n.nodeId === 'agent')?.error).toBe('Stopped by user')
  })

  it('frees the trigger so the workflow can be run again right away', async () => {
    const wf = makeWorkflow()
    mockState.config.workflows = [wf]
    const runPromise = executeWorkflow(wf)
    await nextSession()
    await vi.advanceTimersByTimeAsync(0)

    const runId = [...mockState.workflowExecutions.keys()][0]
    await stopWorkflowRun(runId)
    await runPromise

    const second = executeWorkflow(wf)
    const sessionId = await nextSession()
    emitExit(sessionId, 0)
    expect((await second).status).toBe('success')
  })
})

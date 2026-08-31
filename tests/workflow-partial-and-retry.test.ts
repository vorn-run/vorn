// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { WorkflowDefinition, WorkflowExecution } from '../src/shared/types'

type ExitListener = (p: { id: string; exitCode: number }) => void

const exitListeners = new Set<ExitListener>()

function emitExit(id: string, exitCode: number): void {
  for (const l of [...exitListeners]) l({ id, exitCode })
}

const claims = new Map<string, string>()
let runSeq = 0
let sessionSeq = 0

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

/** Remembers which prompt each session was launched with, keyed by session id. */
const launchedPrompts = new Map<string, string>()
let onSessionCreated: ((id: string) => void) | null = null

const createHeadlessSession = vi.fn((opts: { initialPrompt?: string }) => {
  const id = `sess-${++sessionSeq}`
  launchedPrompts.set(id, opts.initialPrompt ?? '')
  queueMicrotask(() => onSessionCreated?.(id))
  return Promise.resolve({
    id,
    pid: 4242,
    launchCommand: 'claude --dangerously-skip-permissions -p',
    agentSessionId: undefined,
    worktreePath: undefined
  })
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

const { executeWorkflow, retryRunFromFailure, rerunWorkflowRun, contextFromRun } =
  await import('../src/renderer/lib/workflow-execution')

const agentNode = (id: string, prompt: string) => ({
  id,
  type: 'launchAgent' as const,
  label: id,
  slug: id,
  position: { x: 0, y: 0 },
  config: {
    agentType: 'claude',
    projectName: 'p',
    projectPath: '/p',
    headless: true,
    prompt
  }
})

/** trigger → a → b → c */
function makeChain(): WorkflowDefinition {
  return {
    id: 'wf-chain',
    name: 'Chain',
    icon: 'Rocket',
    enabled: true,
    nodes: [
      { id: 'trigger', type: 'trigger', label: 'Trigger', position: { x: 0, y: 0 }, config: {} },
      agentNode('a', 'first'),
      agentNode('b', 'second'),
      agentNode('c', 'third')
    ],
    edges: [
      { id: 'e1', source: 'trigger', target: 'a' },
      { id: 'e2', source: 'a', target: 'b' },
      { id: 'e3', source: 'b', target: 'c' }
    ]
  } as unknown as WorkflowDefinition
}

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
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  ;(globalThis as any).Notification = { permission: 'denied' }
  exitListeners.clear()
  claims.clear()
  launchedPrompts.clear()
  runSeq = 0
  sessionSeq = 0
  onSessionCreated = null
  mockState.workflowExecutions.clear()

  globalThis.window.api = {
    claimWorkflowRun,
    releaseWorkflowRun,
    createHeadlessSession,
    killHeadlessSession: vi.fn(() => Promise.resolve()),
    saveWorkflowRun: vi.fn(() => Promise.resolve()),
    reportWorkflowComplete: vi.fn(() => Promise.resolve()),
    completeConnectorInbox: vi.fn(() => Promise.resolve()),
    renewConnectorInbox: vi.fn(() => Promise.resolve(true)),
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

describe('running up to one step', () => {
  it('executes only the target and its upstream slice', async () => {
    const wf = makeChain()
    const runPromise = executeWorkflow(wf, undefined, { targetNodeId: 'b' })

    emitExit(await nextSession(), 0)
    emitExit(await nextSession(), 0)

    const execution = await runPromise
    expect(execution.partial).toBe(true)
    expect(execution.status).toBe('success')
    expect(execution.nodeStates.find((n) => n.nodeId === 'a')?.status).toBe('success')
    expect(execution.nodeStates.find((n) => n.nodeId === 'b')?.status).toBe('success')
    expect(execution.nodeStates.find((n) => n.nodeId === 'c')?.status).toBe('skipped')
    expect(createHeadlessSession).toHaveBeenCalledTimes(2)
  })

  it('claims separately from a full run, so it never blocks one', async () => {
    const wf = makeChain()
    const partialPromise = executeWorkflow(wf, undefined, { targetNodeId: 'a' })
    emitExit(await nextSession(), 0)
    await partialPromise
    expect(claimWorkflowRun).toHaveBeenCalledWith(
      expect.objectContaining({ params: expect.stringContaining(':target:a') })
    )
  })
})

describe('retrying a failed run', () => {
  async function failAtB(): Promise<{ wf: WorkflowDefinition; failed: WorkflowExecution }> {
    const wf = makeChain()
    const runPromise = executeWorkflow(wf)
    emitExit(await nextSession(), 0)
    emitExit(await nextSession(), 1)
    const failed = await runPromise
    expect(failed.status).toBe('error')
    return { wf, failed }
  }

  it('adopts completed outputs and restarts at the failure', async () => {
    const { wf, failed } = await failAtB()
    createHeadlessSession.mockClear()

    const retryPromise = retryRunFromFailure(wf, failed)
    emitExit(await nextSession(), 0)
    emitExit(await nextSession(), 0)

    const retried = await retryPromise
    expect(retried.status).toBe('success')
    expect(retried.retryOfRunId).toBe(failed.runId)
    // Step a is adopted, never relaunched: only b and c run.
    expect(createHeadlessSession).toHaveBeenCalledTimes(2)
    const prompts = [...launchedPrompts.values()]
    expect(prompts.filter((p) => p.includes('first'))).toHaveLength(1)
    expect(retried.nodeStates.find((n) => n.nodeId === 'a')?.status).toBe('success')
  })

  it('re-runs a run from scratch with its original context', async () => {
    const { wf, failed } = await failAtB()
    createHeadlessSession.mockClear()

    const rerunPromise = rerunWorkflowRun(wf, failed)
    emitExit(await nextSession(), 0)
    emitExit(await nextSession(), 0)
    emitExit(await nextSession(), 0)

    const rerun = await rerunPromise
    expect(rerun.status).toBe('success')
    expect(rerun.runId).not.toBe(failed.runId)
    expect(createHeadlessSession).toHaveBeenCalledTimes(3)
  })

  it('rebuilds a context that never carries the inbox lease', () => {
    const run = {
      runId: 'r',
      workflowId: 'wf',
      startedAt: 's',
      status: 'error',
      nodeStates: [],
      inputs: { pr: '7' },
      connectorItem: {
        inboxId: 9,
        inboxLeaseToken: 'lease',
        connectionId: 'c',
        connectorId: 'github',
        externalId: '1',
        title: 't',
        raw: {}
      }
    } as unknown as WorkflowExecution
    const context = contextFromRun(run)
    expect(context?.inputs).toEqual({ pr: '7' })
    expect(context?.connectorItem?.inboxId).toBeUndefined()
    expect(context?.connectorItem?.inboxLeaseToken).toBeUndefined()
    expect(context?.connectorItem?.externalId).toBe('1')
  })
})

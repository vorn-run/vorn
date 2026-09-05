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
type DataListener = (p: { id: string; data: string }) => void

const exitListeners = new Set<ExitListener>()
const dataListeners = new Set<DataListener>()

function emitExit(id: string, exitCode: number): void {
  for (const l of [...exitListeners]) l({ id, exitCode })
}

function emitData(id: string, data: string): void {
  for (const l of [...dataListeners]) l({ id, data })
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

const createHeadlessSession = vi.fn((_opts: { initialPrompt?: string }) => {
  const id = `sess-${++sessionSeq}`
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

const {
  adoptConnectorInboxLease,
  approveWorkflowGate,
  executeWorkflow,
  reconcileRunningExecutions,
  rejectWorkflowGate,
  stopWorkflowRun
} = await import('../src/renderer/lib/workflow-execution')

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
  dataListeners.clear()
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
    completeConnectorInbox: vi.fn(() => Promise.resolve()),
    renewConnectorInbox: vi.fn(() => Promise.resolve(true)),
    runWorkflowManual: vi.fn(() => Promise.resolve()),
    listSessionEventsBySession: vi.fn(() => Promise.resolve([])),
    getWorktreeActiveSessions: vi.fn(() => Promise.resolve({ count: 0 })),
    isWorktreeDirty: vi.fn(() => Promise.resolve(false)),
    removeWorktree: vi.fn(() => Promise.resolve()),
    onHeadlessData: (fn: DataListener) => {
      dataListeners.add(fn)
      return () => dataListeners.delete(fn)
    },
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
  it('acknowledges a durable connector event only after its workflow succeeds', async () => {
    const wf = makeWorkflow()
    const run = executeWorkflow(wf, {
      connectorItem: {
        inboxId: 73,
        inboxLeaseToken: 'lease-73',
        connectionId: 'conn-1',
        connectorId: 'github',
        externalId: 'issue-7',
        title: 'A',
        raw: {}
      }
    })
    const sessionId = await nextSession()

    expect(window.api.completeConnectorInbox).not.toHaveBeenCalled()
    emitExit(sessionId, 0)
    await vi.runAllTimersAsync()
    expect((await run).status).toBe('success')
    expect(window.api.completeConnectorInbox).toHaveBeenCalledWith({
      id: 73,
      leaseToken: 'lease-73',
      disposition: 'processed'
    })
  })

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

  it('runs in parallel for manual triggers started with different inputs', async () => {
    const wf = makeWorkflow()
    const ids: string[] = []
    onSessionCreated = (id) => ids.push(id)

    const runA = executeWorkflow(wf, { inputs: { issue: 'gh-7' } })
    const runB = executeWorkflow(wf, { inputs: { issue: 'gh-8' } })

    await vi.advanceTimersByTimeAsync(0)
    expect(ids).toHaveLength(2)

    ids.forEach((id) => emitExit(id, 0))
    const [a, b] = await Promise.all([runA, runB])

    expect(a.runId).not.toBe(b.runId)
    expect(a.inputs).toEqual({ issue: 'gh-7' })
    expect(b.inputs).toEqual({ issue: 'gh-8' })
  })

  it('runs in parallel when one context is launched with different inputs', async () => {
    // The context alone used to decide the fingerprint, so two runs from the
    // same card with different answers collapsed into one.
    const wf = makeWorkflow()
    const ids: string[] = []
    onSessionCreated = (id) => ids.push(id)

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const task = { id: 'task-1', title: 'T' } as any
    const runA = executeWorkflow(wf, { task, inputs: { issue: 'gh-7' } })
    const runB = executeWorkflow(wf, { task, inputs: { issue: 'gh-8' } })

    await vi.advanceTimersByTimeAsync(0)
    expect(ids).toHaveLength(2)

    ids.forEach((id) => emitExit(id, 0))
    const [a, b] = await Promise.all([runA, runB])
    expect(a.runId).not.toBe(b.runId)
  })

  it('still collapses a double-fire of one context with identical inputs', async () => {
    const wf = makeWorkflow()
    const ids: string[] = []
    onSessionCreated = (id) => ids.push(id)

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const task = { id: 'task-2', title: 'T' } as any
    const runA = executeWorkflow(wf, { task, inputs: { issue: 'gh-7' } })
    const runB = executeWorkflow(wf, { task, inputs: { issue: 'gh-7' } })

    await vi.advanceTimersByTimeAsync(0)
    expect(ids).toHaveLength(1)

    ids.forEach((id) => emitExit(id, 0))
    const [a, b] = await Promise.all([runA, runB])
    expect(a.runId).toBe(b.runId)
  })

  it('substitutes run inputs into the agent prompt it launches', async () => {
    const wf = makeWorkflow('wf-tmpl')
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ;(wf.nodes[1].config as any).prompt = 'Review PR {{inputs.pr_number}} in {{inputs.repo}}'
    mockState.config.workflows = [wf]

    const run = executeWorkflow(wf, { inputs: { pr_number: 42, repo: 'vorn-run/vorn' } })
    const sess = await nextSession()
    // Drive the run to completion before asserting: a throw here would
    // otherwise leave the run pending and strand the shared fake timers.
    emitData(sess, 'done')
    emitExit(sess, 0)
    await vi.runAllTimersAsync()
    await run

    const launch = createHeadlessSession.mock.calls.at(-1)?.[0]
    expect(launch?.initialPrompt).toContain('Review PR 42 in vorn-run/vorn')
    expect(launch?.initialPrompt).not.toContain('{{inputs.')
  })

  it('collapses two manual triggers carrying the same inputs into a single run', async () => {
    const wf = makeWorkflow()
    const ids: string[] = []
    onSessionCreated = (id) => ids.push(id)

    // Key order differs, but the parameters are the same trigger.
    const first = executeWorkflow(wf, { inputs: { a: '1', b: '2' } })
    await vi.advanceTimersByTimeAsync(0)
    const second = executeWorkflow(wf, { inputs: { b: '2', a: '1' } })
    await vi.advanceTimersByTimeAsync(0)

    expect(ids).toHaveLength(1)
    ids.forEach((id) => emitExit(id, 0))

    const [a, b] = await Promise.all([first, second])
    expect(b.runId).toBe(a.runId)
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
    const runPromise = executeWorkflow(wf, {
      connectorItem: {
        inboxId: 74,
        inboxLeaseToken: 'lease-74',
        connectionId: 'conn-1',
        connectorId: 'github',
        externalId: 'issue-8',
        title: 'B',
        raw: {}
      }
    })

    const sessionId = await nextSession()
    await vi.advanceTimersByTimeAsync(0)

    const runId = [...mockState.workflowExecutions.keys()][0]
    await stopWorkflowRun(runId)

    const execution = await runPromise
    expect(killHeadlessSession).toHaveBeenCalledWith(sessionId)
    expect(execution.status).toBe('cancelled')
    expect(execution.nodeStates.find((n) => n.nodeId === 'agent')?.error).toBe('Stopped by user')
    expect(window.api.completeConnectorInbox).toHaveBeenCalledWith({
      id: 74,
      leaseToken: 'lease-74',
      disposition: 'processed',
      error: 'Workflow stopped by user'
    })
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

describe('rejecting an approval gate', () => {
  it('restores connector context for steps after approval', async () => {
    const base = makeWorkflow()
    const agent = {
      ...base.nodes.find((node) => node.id === 'agent')!,
      config: {
        ...(base.nodes.find((node) => node.id === 'agent')!.config as Record<string, unknown>),
        prompt: 'Handle {{connectorItem.title}}'
      }
    }
    const workflow = {
      ...base,
      nodes: [
        base.nodes.find((node) => node.id === 'trigger')!,
        {
          id: 'approval',
          type: 'approval',
          label: 'Approve',
          position: { x: 0, y: 1 },
          config: {}
        },
        agent
      ],
      edges: [
        { id: 'e1', source: 'trigger', target: 'approval' },
        { id: 'e2', source: 'approval', target: 'agent' }
      ]
    } as unknown as WorkflowDefinition
    mockState.config.workflows = [workflow]
    const waiting = await executeWorkflow(workflow, {
      connectorItem: {
        inboxId: 90,
        inboxLeaseToken: 'lease-90',
        connectionId: 'conn-1',
        connectorId: 'github',
        externalId: 'issue-90',
        title: 'Context survives',
        raw: {}
      }
    })

    const resumed = approveWorkflowGate(waiting, 'approval')
    const sessionId = await nextSession()
    emitExit(sessionId, 0)
    await resumed

    expect(createHeadlessSession.mock.calls.at(-1)?.[0].initialPrompt).toContain(
      'Handle Context survives'
    )
  })

  it('treats explicit rejection as a terminal connector decision', async () => {
    const workflow = {
      ...makeWorkflow(),
      nodes: [
        {
          id: 'trigger',
          type: 'trigger',
          label: 'Trigger',
          position: { x: 0, y: 0 },
          config: {}
        },
        {
          id: 'approval',
          type: 'approval',
          label: 'Approve',
          position: { x: 0, y: 1 },
          config: {}
        }
      ],
      edges: [{ id: 'e1', source: 'trigger', target: 'approval' }]
    } as unknown as WorkflowDefinition
    mockState.config.workflows = [workflow]

    const waiting = await executeWorkflow(workflow, {
      connectorItem: {
        inboxId: 91,
        inboxLeaseToken: 'lease-91',
        connectionId: 'conn-1',
        connectorId: 'github',
        externalId: 'issue-91',
        title: 'Needs approval',
        raw: {}
      }
    })
    expect(waiting.nodeStates.find((node) => node.nodeId === 'approval')?.status).toBe('waiting')

    await adoptConnectorInboxLease(waiting, {
      ...waiting.connectorItem!,
      inboxLeaseToken: 'lease-92'
    })
    const rejected = await rejectWorkflowGate(waiting, 'approval')
    expect(rejected.status).toBe('error')
    expect(window.api.completeConnectorInbox).toHaveBeenCalledWith({
      id: 91,
      leaseToken: 'lease-92',
      disposition: 'processed'
    })
  })
})

describe('a step that declared its failure survivable', () => {
  /** The same workflow, with the agent free to fail without ending the run. */
  function makeSurvivableWorkflow(id = 'wf-1'): WorkflowDefinition {
    const wf = makeWorkflow(id)
    const agent = wf.nodes.find((n) => n.id === 'agent')!
    return { ...wf, nodes: [wf.nodes[0], { ...agent, onError: 'continue' }] } as WorkflowDefinition
  }

  it('leaves the run successful, and the step itself failed', async () => {
    const wf = makeSurvivableWorkflow()
    onSessionCreated = (id) => {
      onSessionCreated = null
      emitExit(id, 1)
    }

    const execution = await executeWorkflow(wf)

    expect(execution.status).toBe('success')
    const agent = execution.nodeStates.find((n) => n.nodeId === 'agent')
    expect(agent?.status).toBe('error')
    expect(agent?.error).toBe('Exit code 1')
  })

  it('still ends the run when the step says a failure stops it', async () => {
    const wf = makeWorkflow()
    onSessionCreated = (id) => {
      onSessionCreated = null
      emitExit(id, 1)
    }

    const execution = await executeWorkflow(wf)

    expect(execution.status).toBe('error')
  })

  it('still fails the run for a step that never ran, whatever policy it declared', async () => {
    // "Carry on anyway" speaks for a failure. A step left unreachable never
    // failed, so nothing it declared has anything to say about the run.
    const wf = makeWorkflow('wf-orphan')
    const orphan = {
      ...wf.nodes[1],
      id: 'orphan',
      label: 'Orphan',
      onError: 'continue'
    } as WorkflowDefinition['nodes'][number]
    const withOrphan = { ...wf, nodes: [...wf.nodes, orphan] } as WorkflowDefinition
    onSessionCreated = (id) => {
      onSessionCreated = null
      emitExit(id, 0)
    }

    const execution = await executeWorkflow(withOrphan)

    expect(execution.status).toBe('error')
    const left = execution.nodeStates.find((n) => n.nodeId === 'orphan')
    expect(left?.error).toMatch(/^Skipped:/)
  })

  it('fails the run when a loop that carries on holds a step that does not', async () => {
    // The loop excuses its own failure; the body step that stopped still had
    // the last word about the run.
    const wf = makeWorkflow('wf-loop')
    const body = {
      ...wf.nodes[1],
      id: 'body',
      label: 'Body'
    } as WorkflowDefinition['nodes'][number]
    const loop = {
      id: 'loop',
      type: 'loop',
      label: 'Until it is clean',
      position: { x: 0, y: 1 },
      onError: 'continue',
      config: { nodeType: 'loop', bodyNodeIds: ['body'], maxIterations: 1 }
    } as unknown as WorkflowDefinition['nodes'][number]
    const looping = {
      ...wf,
      nodes: [wf.nodes[0], loop, body],
      edges: [{ id: 'e1', source: 'trigger', target: 'loop' }]
    } as unknown as WorkflowDefinition
    onSessionCreated = (id) => {
      onSessionCreated = null
      emitExit(id, 1)
    }

    const execution = await executeWorkflow(looping)

    expect(execution.status).toBe('error')
    expect(execution.nodeStates.find((n) => n.nodeId === 'loop')?.status).toBe('error')
    expect(execution.nodeStates.find((n) => n.nodeId === 'body')?.status).toBe('error')
  })

  it('takes neither branch of a condition that failed', async () => {
    const wf = makeWorkflow('wf-condition')
    const agent = wf.nodes[1]
    const condition = {
      id: 'condition',
      type: 'condition',
      label: 'Is it clean',
      position: { x: 0, y: 1 },
      onError: 'continue',
      // A definition that arrived malformed: resolving this throws, so the
      // condition answers nothing.
      config: {
        nodeType: 'condition',
        variable: 1 as unknown as string,
        operator: 'equals',
        value: 'x'
      }
    } as unknown as WorkflowDefinition['nodes'][number]
    const branching = {
      ...wf,
      nodes: [
        wf.nodes[0],
        condition,
        { ...agent, id: 'yes', label: 'Yes' },
        { ...agent, id: 'no', label: 'No' }
      ],
      edges: [
        { id: 'e1', source: 'trigger', target: 'condition' },
        { id: 'e2', source: 'condition', target: 'yes', conditionBranch: 'true' },
        { id: 'e3', source: 'condition', target: 'no', conditionBranch: 'false' }
      ]
    } as unknown as WorkflowDefinition
    let launched = 0
    // Exits whatever starts, so a branch taken by mistake fails the assertion
    // rather than hanging the run.
    onSessionCreated = (id) => {
      launched += 1
      emitExit(id, 0)
    }

    const execution = await executeWorkflow(branching)

    expect(execution.nodeStates.find((n) => n.nodeId === 'condition')?.status).toBe('error')
    expect(execution.nodeStates.find((n) => n.nodeId === 'yes')?.status).toBe('skipped')
    expect(execution.nodeStates.find((n) => n.nodeId === 'no')?.status).toBe('skipped')
    expect(launched).toBe(0)
  })

  it('reads the same way for a run closed after a reload', async () => {
    mockState.config.workflows = [makeSurvivableWorkflow('wf-reloaded')]
    const execution: WorkflowExecution = {
      runId: 'run-reloaded',
      workflowId: 'wf-reloaded',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'running',
      nodeStates: [
        { nodeId: 'trigger', status: 'success' },
        { nodeId: 'agent', status: 'running', sessionId: 'sess-reloaded' }
      ]
    }
    globalThis.window.api.listSessionEventsBySession = vi.fn(
      () =>
        Promise.resolve([
          {
            eventType: 'exited',
            timestamp: '2026-04-20T10:01:00Z',
            metadata: { exitCode: 1 }
          }
        ])
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ) as any

    await reconcileRunningExecutions([execution], mockState.config.workflows)

    expect(execution.status).toBe('success')
    expect(execution.nodeStates.find((n) => n.nodeId === 'agent')?.status).toBe('error')
  })
})

describe('connector run recovery', () => {
  it('acknowledges a terminal run restored after persistence', async () => {
    const execution: WorkflowExecution = {
      runId: 'run-restored',
      workflowId: 'wf-restored',
      startedAt: '2026-04-20T10:00:00Z',
      completedAt: '2026-04-20T10:01:00Z',
      status: 'success',
      connectorInboxId: 101,
      connectorInboxLeaseToken: 'lease-101',
      nodeStates: [{ nodeId: 'agent', status: 'success' }]
    }

    await reconcileRunningExecutions([execution], [])

    expect(window.api.completeConnectorInbox).toHaveBeenCalledWith({
      id: 101,
      leaseToken: 'lease-101',
      disposition: 'processed'
    })
  })
})

describe('step diagnostics', () => {
  /** The engine's own account of the step, distinct from the agent's output. */
  function diagnosticsOf(execution: WorkflowExecution): string {
    return execution.nodeStates.find((n) => n.nodeId === 'agent')?.diagnostics ?? ''
  }

  it('records what was launched, including the exact command', async () => {
    const wf = makeWorkflow()
    const runPromise = executeWorkflow(wf)
    const sessionId = await nextSession()
    emitExit(sessionId, 0)

    const diag = diagnosticsOf(await runPromise)
    expect(diag).toContain('Launching claude')
    expect(diag).toContain(sessionId)
    expect(diag).toContain('pid 4242')
    // Without the command line there is no way to tell a bad flag from a bad agent.
    expect(diag).toContain('claude --dangerously-skip-permissions -p')
  })

  it('distinguishes a silent timeout from a slow one', async () => {
    const wf = makeWorkflow()
    mockState.config.defaults.headlessStepTimeoutMinutes = 1
    const runPromise = executeWorkflow(wf)
    await nextSession()
    await vi.advanceTimersByTimeAsync(60_000 + 10)

    const execution = await runPromise
    const agent = execution.nodeStates.find((n) => n.nodeId === 'agent')
    // Silence is the diagnostic: it means the agent never really ran.
    expect(agent?.error).toMatch(/never produced any output/i)
    expect(diagnosticsOf(execution)).toMatch(/never produced any output/i)
  })

  it('reports how much the agent said when a timeout follows real output', async () => {
    const wf = makeWorkflow()
    mockState.config.defaults.headlessStepTimeoutMinutes = 1
    const runPromise = executeWorkflow(wf)
    const sessionId = await nextSession()
    await vi.advanceTimersByTimeAsync(0)
    emitData(sessionId, 'thinking hard\n')
    await vi.advanceTimersByTimeAsync(60_000 + 10)

    const execution = await runPromise
    const agent = execution.nodeStates.find((n) => n.nodeId === 'agent')
    expect(agent?.error).toContain('14 bytes of output')
    expect(agent?.error).not.toMatch(/never produced any output/i)
    expect(diagnosticsOf(execution)).toContain('First output from the agent')
  })

  it('keeps the timeline out of the agent log', async () => {
    const wf = makeWorkflow()
    const runPromise = executeWorkflow(wf)
    const sessionId = await nextSession()
    await vi.advanceTimersByTimeAsync(0)
    emitData(sessionId, '{"ok":true}')
    emitExit(sessionId, 0)

    const execution = await runPromise
    const agent = execution.nodeStates.find((n) => n.nodeId === 'agent')
    // A typed step parses `logs` for its declared payload, so engine notes must
    // never land there.
    expect(agent?.logs).toBe('{"ok":true}')
    expect(agent?.logs).not.toContain('Launching')
    expect(agent?.diagnostics).toContain('Launching')
  })

  it('notes the exit code and that nothing was produced', async () => {
    const wf = makeWorkflow()
    const runPromise = executeWorkflow(wf)
    const sessionId = await nextSession()
    emitExit(sessionId, 1)

    const diag = diagnosticsOf(await runPromise)
    expect(diag).toContain('exited with code 1')
    expect(diag).toContain('produced nothing at all')
  })

  it('explains a step that never got as far as launching', async () => {
    const wf = makeWorkflow()
    createHeadlessSession.mockImplementationOnce(() =>
      Promise.reject(new Error('git worktree add failed'))
    )

    const execution = await executeWorkflow(wf)
    const agent = execution.nodeStates.find((n) => n.nodeId === 'agent')
    expect(agent?.status).toBe('error')
    // The timeline survives the throw path, so it still says how far it got.
    expect(agent?.diagnostics).toContain('Could not start: git worktree add failed')
  })
})

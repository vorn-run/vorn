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
  headlessSessions: [] as { id: string; agentSessionId?: string }[],
  getNextTask: vi.fn()
}

/** Everything the engine reaches the rest of the server through. */
const hostApi: Record<string, unknown> = {}

vi.mock('../packages/server/src/workflows/host', () => ({
  api: new Proxy(
    {},
    {
      get: (_t, name: string) => hostApi[name]
    }
  ),
  config: () => mockState.config,
  publishRun: (execution: WorkflowExecution) => {
    mockState.workflowExecutions.set(execution.runId, { ...execution })
  },
  runById: (runId: string) => mockState.workflowExecutions.get(runId),
  activeTerminals: () => [],
  activeHeadless: () => mockState.headlessSessions,
  nextTask: (project: string) => mockState.getNextTask(project),
  onHeadlessData: (fn: DataListener) => {
    dataListeners.add(fn)
    return () => dataListeners.delete(fn)
  },
  onHeadlessExit: (fn: ExitListener) => {
    exitListeners.add(fn)
    return () => exitListeners.delete(fn)
  },
  onScriptData: () => () => {}
}))

vi.mock('../packages/server/src/workflows/tasks', () => ({
  startTask: vi.fn(),
  reopenTask: vi.fn()
}))

vi.mock('../packages/server/src/database', () => ({
  listWorkflowRuns: () => [],
  getWorkflowRun: () => null
}))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

const {
  adoptConnectorInboxLease,
  applyGateDecision,
  gateEditIsRefused,
  retryRunFromFailure,
  approveWorkflowGate,
  executeWorkflow,
  resumeSignInWaits,
  reconcileRunningExecutions,
  rejectWorkflowGate,
  requestGateChanges,
  stopWorkflowRun
} = await import('../packages/server/src/workflows/engine')

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

  Object.assign(hostApi, {
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
    removeWorktree: vi.fn(() => Promise.resolve())
  })
})

afterEach(() => {
  vi.useRealTimers()
})

describe('handing a run back before it is finished', () => {
  it('answers with the run as soon as it exists, and keeps walking behind that', async () => {
    // A run lasts as long as its agents do. Anything holding a request open --
    // the CLI, a phone -- wants the run now, not when it ends.
    const wf = makeWorkflow()
    let started: WorkflowExecution | undefined

    const finished = executeWorkflow(wf, undefined, {
      source: 'manual',
      onStarted: (execution) => {
        started = execution
      }
    })

    const sessionId = await nextSession()
    expect(started?.runId).toBe('run-1')
    expect(started?.status).toBe('running')

    emitExit(sessionId, 0)
    expect((await finished).status).toBe('success')
  })
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

    expect(hostApi.completeConnectorInbox).not.toHaveBeenCalled()
    emitExit(sessionId, 0)
    await vi.runAllTimersAsync()
    expect((await run).status).toBe('success')
    expect(hostApi.completeConnectorInbox).toHaveBeenCalledWith({
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

  it('starts beside a run parked on an approval gate', async () => {
    // The gate used to hold the workflow's queue position, so a second run
    // could not begin until someone answered the first.
    const base = makeWorkflow('wf-gate')
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
        base.nodes.find((node) => node.id === 'agent')!
      ],
      edges: [
        { id: 'e1', source: 'trigger', target: 'approval' },
        { id: 'e2', source: 'approval', target: 'agent' }
      ]
    } as unknown as WorkflowDefinition
    mockState.config.workflows = [workflow]

    const parked = await executeWorkflow(workflow, { inputs: { issue: 'gh-7' } })
    expect(parked.nodeStates.find((n) => n.nodeId === 'approval')?.status).toBe('waiting')

    const second = await executeWorkflow(workflow, { inputs: { issue: 'gh-8' } })

    expect(second.runId).not.toBe(parked.runId)
    expect(second.nodeStates.find((n) => n.nodeId === 'approval')?.status).toBe('waiting')
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
    expect(hostApi.completeConnectorInbox).toHaveBeenCalledWith({
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
    expect(hostApi.completeConnectorInbox).toHaveBeenCalledWith({
      id: 91,
      leaseToken: 'lease-92',
      disposition: 'processed'
    })
  })
})

describe('sending the work back from a gate', () => {
  const agent = (id: string, prompt: string) => ({
    id,
    slug: id,
    type: 'launchAgent',
    label: id,
    position: { x: 0, y: 0 },
    config: { agentType: 'claude', projectName: 'p', projectPath: '/p', headless: true, prompt }
  })

  function reviewedWorkflow(): WorkflowDefinition {
    return {
      ...makeWorkflow('wf-review'),
      nodes: [
        { id: 'trigger', type: 'trigger', label: 'Trigger', position: { x: 0, y: 0 }, config: {} },
        agent('draft', 'Write it. Reviewer said: {{steps.approve.feedback}}'),
        agent('polish', 'Polish it'),
        {
          id: 'approve',
          slug: 'approve',
          type: 'approval',
          label: 'Approve',
          position: { x: 0, y: 3 },
          config: {
            message: 'Post this: {{steps.polish.output}}',
            feedback: { from: 'draft', maxRounds: 2 }
          }
        },
        agent('post', 'Post it')
      ],
      edges: [
        { id: 'e1', source: 'trigger', target: 'draft' },
        { id: 'e2', source: 'draft', target: 'polish' },
        { id: 'e3', source: 'polish', target: 'approve' },
        { id: 'e4', source: 'approve', target: 'post' }
      ]
    } as unknown as WorkflowDefinition
  }

  // Ends a step with this output, already listening for the session it starts next.
  async function finishStep(
    session: Promise<string>,
    output: string
  ): Promise<{ next: Promise<string> }> {
    const id = await session
    const next = nextSession()
    emitData(id, output)
    emitExit(id, 0)
    // Not runAllTimers: that would also fire the next step's hour-long timeout.
    await vi.advanceTimersByTimeAsync(5_000)
    return { next }
  }

  it('redoes the steps from the chosen one with the comment, asks again, and stops at the last round', async () => {
    const workflow = reviewedWorkflow()
    mockState.config.workflows = [workflow]

    const first = nextSession()
    const started = executeWorkflow(workflow)
    const { next: polish } = await finishStep(first, 'first draft')
    await finishStep(polish, 'polished one')
    const waiting = await started
    const gate = () => waiting.nodeStates.find((n) => n.nodeId === 'approve')!
    expect(gate()).toMatchObject({ status: 'waiting', round: 1 })
    expect(gate().message).toContain('Post this: polished one')

    const redraft = nextSession()
    const resumed = requestGateChanges(waiting, 'approve', 'Too neat')
    await redraft
    expect(createHeadlessSession.mock.calls.at(-1)?.[0].initialPrompt).toContain(
      'Reviewer said: Too neat'
    )
    const { next: repolish } = await finishStep(redraft, 'second draft')
    await finishStep(repolish, 'polished two')
    await resumed

    expect(gate()).toMatchObject({
      status: 'waiting',
      round: 2,
      feedback: [expect.objectContaining({ round: 1, decision: 'changes', comment: 'Too neat' })]
    })
    expect(gate().message).toContain('Post this: polished two')
    // Draft and polish twice; the step after the gate never ran.
    expect(createHeadlessSession).toHaveBeenCalledTimes(4)

    await requestGateChanges(waiting, 'approve', 'Once more')
    expect(gate()).toMatchObject({ status: 'waiting', round: 2 })
    expect(createHeadlessSession).toHaveBeenCalledTimes(4)

    const rejected = await rejectWorkflowGate(waiting, 'approve', { note: 'Not today' })
    expect(rejected.status).toBe('error')
    expect(gate()).toMatchObject({
      status: 'error',
      error: 'Not today',
      rejectedAt: expect.any(String)
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
    hostApi.listSessionEventsBySession = vi.fn(() =>
      Promise.resolve([
        {
          eventType: 'exited',
          timestamp: '2026-04-20T10:01:00Z',
          metadata: { exitCode: 1 }
        }
      ])
    )

    await reconcileRunningExecutions([execution], mockState.config.workflows)

    expect(execution.status).toBe('success')
    expect(execution.nodeStates.find((n) => n.nodeId === 'agent')?.status).toBe('error')
  })

  it('fails the run for a step the reconciler found no session for, whatever it declared', async () => {
    // Nothing was recorded of it starting, so "carry on anyway" speaks for a
    // failure that never happened.
    mockState.config.workflows = [makeSurvivableWorkflow('wf-abandoned')]
    const execution: WorkflowExecution = {
      runId: 'run-abandoned',
      workflowId: 'wf-abandoned',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'running',
      nodeStates: [
        { nodeId: 'trigger', status: 'success' },
        { nodeId: 'agent', status: 'running' }
      ]
    }

    await reconcileRunningExecutions([execution], mockState.config.workflows)

    expect(execution.status).toBe('error')
    const agent = execution.nodeStates.find((n) => n.nodeId === 'agent')
    expect(agent?.status).toBe('error')
    expect(agent?.error).toBe('Run abandoned (no session id recorded)')
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

    expect(hostApi.completeConnectorInbox).toHaveBeenCalledWith({
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

describe('a step whose connection signed out', () => {
  function postWorkflow(): WorkflowDefinition {
    return {
      id: 'wf-post',
      name: 'Post to Substack',
      icon: 'Rocket',
      enabled: true,
      nodes: [
        { id: 'trigger', type: 'trigger', label: 'Trigger', position: { x: 0, y: 0 }, config: {} },
        {
          id: 'draft',
          type: 'callConnectorAction',
          label: 'Draft',
          position: { x: 0, y: 1 },
          config: { connectionId: 'conn-sub', action: 'createDraft', args: {} }
        },
        {
          id: 'tell',
          type: 'callConnectorAction',
          label: 'Tell',
          position: { x: 0, y: 2 },
          config: { connectionId: 'conn-other', action: 'post', args: {} }
        }
      ],
      edges: [
        { id: 'e1', source: 'trigger', target: 'draft' },
        { id: 'e2', source: 'draft', target: 'tell' }
      ]
    } as unknown as WorkflowDefinition
  }

  const signedOut = {
    success: false,
    error: 'Substack was signed out. Sign in again, and this step runs again.',
    errorKind: 'needs-sign-in'
  }
  const stateOf = (run: WorkflowExecution, nodeId: string) =>
    run.nodeStates.find((ns) => ns.nodeId === nodeId)

  it('waits for the sign-in instead of failing, and runs nothing after it', async () => {
    const workflow = postWorkflow()
    mockState.config.workflows = [workflow]
    hostApi.executeConnectorAction = vi.fn().mockResolvedValue(signedOut)
    const run = await executeWorkflow(workflow)
    expect(stateOf(run, 'draft')).toMatchObject({ status: 'waiting', waitingFor: 'signIn' })
    expect(stateOf(run, 'tell')?.status).toBe('pending')
    expect(run.status).toBe('running')
  })

  it('cannot be approved past, since only signing in again ends it', async () => {
    const workflow = postWorkflow()
    mockState.config.workflows = [workflow]
    const execute = vi.fn().mockResolvedValue(signedOut)
    hostApi.executeConnectorAction = execute
    const run = await executeWorkflow(workflow)
    await applyGateDecision(run.runId, 'draft', 'approve')
    expect(stateOf(mockState.workflowExecutions.get(run.runId)!, 'draft')?.status).toBe('waiting')
    expect(execute).toHaveBeenCalledTimes(1)
  })

  it('fails inside a loop, which cannot wait, and says to run the workflow again', async () => {
    const base = postWorkflow()
    const loop = {
      id: 'loop',
      type: 'loop',
      label: 'Each draft',
      position: { x: 0, y: 1 },
      config: { nodeType: 'loop', bodyNodeIds: ['draft'], maxIterations: 1 }
    }
    const workflow = {
      ...base,
      id: 'wf-post-loop',
      nodes: [base.nodes[0], loop, base.nodes[1]],
      edges: [{ id: 'e1', source: 'trigger', target: 'loop' }]
    } as unknown as WorkflowDefinition
    mockState.config.workflows = [workflow]
    hostApi.executeConnectorAction = vi.fn().mockResolvedValue(signedOut)
    const run = await executeWorkflow(workflow)
    expect(stateOf(run, 'draft')).toMatchObject({ status: 'error' })
    expect(stateOf(run, 'draft')?.waitingFor).toBeUndefined()
    expect(stateOf(run, 'draft')?.error).toMatch(/inside a loop/)
    expect(run.status).not.toBe('running')
  })

  it('runs the step again once its connection signs in, and carries on from there', async () => {
    const workflow = postWorkflow()
    mockState.config.workflows = [workflow]
    const execute = vi
      .fn()
      .mockResolvedValueOnce(signedOut)
      .mockResolvedValue({ success: true, output: { id: 7 } })
    hostApi.executeConnectorAction = execute
    const run = await executeWorkflow(workflow)

    await resumeSignInWaits('conn-other', [run])
    expect(execute).toHaveBeenCalledTimes(1)

    await resumeSignInWaits('conn-sub', [run])
    const finished = mockState.workflowExecutions.get(run.runId)!
    expect(stateOf(finished, 'draft')?.status).toBe('success')
    expect(stateOf(finished, 'draft')?.waitingFor).toBeUndefined()
    expect(stateOf(finished, 'tell')?.status).toBe('success')
    expect(execute).toHaveBeenCalledTimes(3)
  })
})

describe('a loop whose steps form a graph', () => {
  type Node = WorkflowDefinition['nodes'][number]
  const action = (id: string, extra: Partial<Node> = {}): Node =>
    ({
      id,
      type: 'callConnectorAction',
      label: id,
      slug: id,
      position: { x: 0, y: 0 },
      config: { nodeType: 'callConnectorAction', connectionId: 'conn', action: id, args: {} },
      ...extra
    }) as unknown as Node
  const condition = (id: string, variable: string, value: string): Node =>
    ({
      id,
      type: 'condition',
      label: id,
      slug: id,
      position: { x: 0, y: 0 },
      config: { variable, operator: 'equals', value }
    }) as unknown as Node
  const loopNode = (bodyNodeIds: string[], config: Record<string, unknown> = {}): Node =>
    ({
      id: 'loop',
      type: 'loop',
      label: 'Loop',
      slug: 'loop',
      position: { x: 0, y: 0 },
      config: { nodeType: 'loop', bodyNodeIds, maxIterations: 1, ...config }
    }) as unknown as Node
  const workflow = (
    nodes: Node[],
    edges: [string, string, ('true' | 'false')?][]
  ): WorkflowDefinition =>
    ({
      id: 'wf-loop-graph',
      name: 'Loop graph',
      icon: 'Rocket',
      enabled: true,
      nodes: [
        { id: 'trigger', type: 'trigger', label: 'Trigger', position: { x: 0, y: 0 }, config: {} },
        ...nodes
      ],
      edges: edges.map(([source, target, conditionBranch], i) => ({
        id: `e${i}`,
        source,
        target,
        ...(conditionBranch && { conditionBranch })
      }))
    }) as unknown as WorkflowDefinition
  const stateOf = (run: WorkflowExecution, nodeId: string) =>
    run.nodeStates.find((ns) => ns.nodeId === nodeId)

  /** Answers each action from `outputs`, and records what ran, in order. */
  function connectorAnswers(outputs: Record<string, unknown> = {}, failing: string[] = []) {
    const calls: { action: string; args: Record<string, unknown> }[] = []
    hostApi.executeConnectorAction = vi.fn(
      async ({ action, args }: { action: string; args: Record<string, unknown> }) => {
        calls.push({ action, args })
        return failing.includes(action)
          ? { success: false, error: `${action} broke` }
          : { success: true, output: outputs[action] ?? {} }
      }
    )
    return calls
  }

  it('takes one branch of a condition inside the body, and runs what follows the loop once', async () => {
    const calls = connectorAnswers({ check: { verdict: 'yes' } })
    const run = await executeWorkflow(
      workflow(
        [
          loopNode(['check', 'decide', 'yes', 'no']),
          action('check'),
          condition('decide', '{{steps.check.verdict}}', 'yes'),
          action('yes'),
          action('no'),
          action('after')
        ],
        [
          ['trigger', 'loop'],
          ['loop', 'check'],
          ['check', 'decide'],
          ['decide', 'yes', 'true'],
          ['decide', 'no', 'false'],
          ['yes', 'after'],
          ['no', 'after']
        ]
      )
    )

    expect(run.status).toBe('success')
    expect(calls.map((c) => c.action)).toEqual(['check', 'yes', 'after'])
    expect(stateOf(run, 'no')).toMatchObject({ status: 'skipped', skipReason: 'branch' })
    expect(stateOf(run, 'loop')?.status).toBe('success')
  })

  it('skips the rest of a failed pass instead of running it outside the loop later', async () => {
    // A failed pass used to leave the rest of the body pending, and the main
    // scheduler then ran it on its own once its predecessor had settled.
    const calls = connectorAnswers({}, ['first'])
    const run = await executeWorkflow(
      workflow(
        [loopNode(['first', 'second']), action('first'), action('second'), action('after')],
        [
          ['trigger', 'loop'],
          ['loop', 'first'],
          ['first', 'second'],
          ['second', 'after']
        ]
      )
    )

    expect(calls.map((c) => c.action)).toEqual(['first'])
    expect(stateOf(run, 'second')?.status).toBe('skipped')
    expect(stateOf(run, 'loop')?.status).toBe('error')
    expect(run.status).toBe('error')
  })

  it('runs the body once per item, each pass reading its own item', async () => {
    const calls = connectorAnswers({
      list: { findings: [{ path: 'a.ts' }, { path: 'b.ts' }, { path: 'c.ts' }] }
    })
    const comment = action('comment')
    ;(comment.config as { args: Record<string, string> }).args = {
      path: '{{loop.item.path}}',
      position: '{{loop.number}} of {{loop.count}}'
    }
    const run = await executeWorkflow(
      workflow(
        [
          action('list'),
          loopNode(['comment'], { mode: 'forEach', items: '{{steps.list.findings}}' }),
          comment,
          action('after')
        ],
        [
          ['trigger', 'list'],
          ['list', 'loop'],
          ['loop', 'comment'],
          ['comment', 'after']
        ]
      )
    )

    expect(run.status).toBe('success')
    expect(calls.filter((c) => c.action === 'comment').map((c) => c.args)).toEqual([
      { path: 'a.ts', position: '1 of 3' },
      { path: 'b.ts', position: '2 of 3' },
      { path: 'c.ts', position: '3 of 3' }
    ])
    expect(calls.at(-1)?.action).toBe('after')
    const loopState = stateOf(run, 'loop')
    expect(loopState).toMatchObject({
      output: '3',
      structuredOutput: expect.objectContaining({ count: 3 })
    })
    const results = (
      loopState?.structuredOutput as {
        results: { item: unknown; steps: Record<string, { status: string }> }[]
      }
    ).results
    expect(results.map((r) => r.item)).toEqual([
      { path: 'a.ts' },
      { path: 'b.ts' },
      { path: 'c.ts' }
    ])
    expect(results[0].steps.comment.status).toBe('success')
  })

  it('branches differently for each item', async () => {
    const calls = connectorAnswers({ list: { items: [{ keep: 'yes' }, { keep: 'no' }] } })
    await executeWorkflow(
      workflow(
        [
          action('list'),
          loopNode(['decide', 'yes', 'no'], { mode: 'forEach', items: '{{steps.list.items}}' }),
          condition('decide', '{{loop.item.keep}}', 'yes'),
          action('yes'),
          action('no')
        ],
        [
          ['trigger', 'list'],
          ['list', 'loop'],
          ['loop', 'decide'],
          ['decide', 'yes', 'true'],
          ['decide', 'no', 'false']
        ]
      )
    )
    expect(calls.map((c) => c.action)).toEqual(['list', 'yes', 'no'])
  })

  it('walks a list with more items than a repeat loop may pass', async () => {
    const many = Array.from({ length: 25 }, (_, i) => ({ n: i }))
    const calls = connectorAnswers({ list: { items: many } })
    const run = await executeWorkflow(
      workflow(
        [
          action('list'),
          loopNode(['each'], { mode: 'forEach', items: '{{steps.list.items}}' }),
          action('each')
        ],
        [
          ['trigger', 'list'],
          ['list', 'loop'],
          ['loop', 'each']
        ]
      )
    )
    expect(calls.filter((c) => c.action === 'each')).toHaveLength(25)
    expect(stateOf(run, 'loop')?.output).toBe('25')
  })

  it('skips the steps inside for an empty list, and carries on', async () => {
    const calls = connectorAnswers({ list: { items: [] } })
    const run = await executeWorkflow(
      workflow(
        [
          action('list'),
          loopNode(['each'], { mode: 'forEach', items: '{{steps.list.items}}' }),
          action('each'),
          action('after')
        ],
        [
          ['trigger', 'list'],
          ['list', 'loop'],
          ['loop', 'each'],
          ['each', 'after']
        ]
      )
    )
    expect(calls.map((c) => c.action)).toEqual(['list', 'after'])
    expect(stateOf(run, 'each')?.status).toBe('skipped')
    expect(run.status).toBe('success')
  })

  it('fails with a reason when its items are not a list', async () => {
    connectorAnswers({ list: { items: 'not a list' } })
    const run = await executeWorkflow(
      workflow(
        [
          action('list'),
          loopNode(['each'], { mode: 'forEach', items: '{{steps.list.items}}' }),
          action('each')
        ],
        [
          ['trigger', 'list'],
          ['list', 'loop'],
          ['loop', 'each']
        ]
      )
    )
    expect(stateOf(run, 'loop')?.error).toMatch(/Not a list.*\{\{steps\.list\.items\}\}/)
    expect(run.status).toBe('error')
  })

  it('stops at the item whose step fails, and reports which', async () => {
    let n = 0
    hostApi.executeConnectorAction = vi.fn(async ({ action: name }: { action: string }) => {
      if (name === 'list') return { success: true, output: { items: [1, 2, 3] } }
      n++
      return n === 2 ? { success: false, error: 'broke' } : { success: true, output: {} }
    })
    const run = await executeWorkflow(
      workflow(
        [
          action('list'),
          loopNode(['each'], { mode: 'forEach', items: '{{steps.list.items}}' }),
          action('each')
        ],
        [
          ['trigger', 'list'],
          ['list', 'loop'],
          ['loop', 'each']
        ]
      )
    )
    expect(n).toBe(2)
    expect(stateOf(run, 'loop')?.error).toBe('"each" failed on item 2')
  })

  it('still caps a repeat loop at ten passes', async () => {
    const calls = connectorAnswers()
    await executeWorkflow(
      workflow(
        [loopNode(['each'], { maxIterations: 50 }), action('each')],
        [
          ['trigger', 'loop'],
          ['loop', 'each']
        ]
      )
    )
    expect(calls).toHaveLength(10)
  })

  it('hands the rows a reviewer kept at a gate to a for-each loop', async () => {
    const findings = [
      { path: 'a.ts', body: 'one' },
      { path: 'b.ts', body: 'two' },
      { path: 'c.ts', body: 'three' }
    ]
    const calls = connectorAnswers({ review: { findings } })
    const gate = {
      id: 'gate',
      type: 'approval',
      label: 'Check the review',
      slug: 'gate',
      position: { x: 0, y: 0 },
      config: { edit: '{{steps.review.findings}}' }
    } as unknown as Node
    const comment = action('comment')
    ;(comment.config as { args: Record<string, string> }).args = { path: '{{loop.item.path}}' }
    const wf = workflow(
      [
        action('review'),
        gate,
        loopNode(['comment'], { mode: 'forEach', items: '{{steps.gate.items}}' }),
        comment
      ],
      [
        ['trigger', 'review'],
        ['review', 'gate'],
        ['gate', 'loop'],
        ['loop', 'comment']
      ]
    )
    mockState.config.workflows = [wf]
    const run = await executeWorkflow(wf)

    // The list reaches the reviewer whole, as readable JSON.
    const asked = stateOf(run, 'gate')?.editableText ?? ''
    expect(JSON.parse(asked)).toEqual(findings)
    expect(asked).toContain('\n  {')

    // A broken rewrite is refused with where it broke.
    expect(gateEditIsRefused(run.runId, 'gate', '[{"path": "a.ts",}]')).toMatch(
      /line 1, column \d+/
    )

    await applyGateDecision(
      run.runId,
      'gate',
      'approve',
      undefined,
      JSON.stringify([findings[0], findings[2]])
    )
    expect(calls.filter((c) => c.action === 'comment').map((c) => c.args.path)).toEqual([
      'a.ts',
      'c.ts'
    ])
  })

  it('retries past a finished loop with its steps still readable', async () => {
    // A retry keeps the loop that finished; what follows it must still read
    // the loop's steps rather than get nothing from them.
    let afterFails = true
    const calls: { action: string; args: Record<string, unknown> }[] = []
    hostApi.executeConnectorAction = vi.fn(
      async ({ action: name, args }: { action: string; args: Record<string, unknown> }) => {
        calls.push({ action: name, args })
        if (name === 'draft') return { success: true, output: { text: 'the draft' } }
        if (name === 'after' && afterFails) return { success: false, error: 'broke' }
        return { success: true, output: {} }
      }
    )
    const after = action('after')
    ;(after.config as { args: Record<string, string> }).args = { text: '{{steps.draft.text}}' }
    const wf = workflow(
      [loopNode(['draft']), action('draft'), after],
      [
        ['trigger', 'loop'],
        ['loop', 'draft'],
        ['draft', 'after']
      ]
    )
    mockState.config.workflows = [wf]
    const failed = await executeWorkflow(wf)
    expect(failed.status).toBe('error')

    afterFails = false
    calls.length = 0
    const retried = await retryRunFromFailure(wf, failed)
    expect(retried.status).toBe('success')
    expect(calls).toEqual([{ action: 'after', args: { text: 'the draft' } }])
  })

  it('refuses a loop whose body is fed from outside it', async () => {
    connectorAnswers()
    const run = await executeWorkflow(
      workflow(
        [loopNode(['inside']), action('outside'), action('inside')],
        [
          ['trigger', 'loop'],
          ['trigger', 'outside'],
          ['outside', 'inside']
        ]
      )
    )
    expect(stateOf(run, 'loop')?.error).toMatch(/from outside it/)
    expect(stateOf(run, 'inside')?.status).toBe('skipped')
  })

  it('skips the body of a loop on a branch not taken, without failing the run', async () => {
    const calls = connectorAnswers({ check: { verdict: 'no' } })
    const run = await executeWorkflow(
      workflow(
        [
          action('check'),
          condition('decide', '{{steps.check.verdict}}', 'yes'),
          loopNode(['inside']),
          action('inside')
        ],
        [
          ['trigger', 'check'],
          ['check', 'decide'],
          ['decide', 'loop', 'true'],
          ['loop', 'inside']
        ]
      )
    )
    expect(calls.map((c) => c.action)).toEqual(['check'])
    expect(stateOf(run, 'loop')).toMatchObject({ status: 'skipped', skipReason: 'branch' })
    expect(stateOf(run, 'inside')).toMatchObject({ status: 'skipped', skipReason: 'branch' })
    expect(run.status).toBe('success')
  })
})

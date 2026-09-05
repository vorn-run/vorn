import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type {
  NodeExecutionState,
  WorkflowDefinition,
  WorkflowExecution
} from '../packages/shared/src/types'

const rpcCall = vi.fn()
const listAllWorkflowRuns = vi.fn()
const dbListWorkflows = vi.fn()

vi.mock('../packages/mcp/src/ws-client', () => ({ rpcCall: (...a: unknown[]) => rpcCall(...a) }))
vi.mock('../packages/mcp/src/data-access', () => ({
  listAllWorkflowRuns: (...a: unknown[]) => listAllWorkflowRuns(...a),
  dbListWorkflows: (...a: unknown[]) => dbListWorkflows(...a),
  listWorkflowRuns: vi.fn(),
  listWorkflowRunsByTask: vi.fn(),
  dbListProjects: vi.fn(),
  dbInsertWorkflow: vi.fn(),
  dbUpdateWorkflow: vi.fn(),
  dbDeleteWorkflow: vi.fn(),
  dbSignalChange: vi.fn()
}))

const { annotateWaitingGates, gateMessage, resolveGateTarget, registerWorkflowTools } =
  await import('../packages/mcp/src/tools/workflows')

type Handler = (args: Record<string, unknown>) => Promise<{
  content: Array<{ type: string; text: string }>
  isError?: boolean
}>

/** Collects the tools the module registers so they can be invoked directly. */
function collect(): Map<string, Handler> {
  const tools = new Map<string, Handler>()
  const server = {
    tool: (name: string, _desc: string, schemaOrHandler: unknown, maybeHandler?: unknown) => {
      tools.set(name, (maybeHandler ?? schemaOrHandler) as Handler)
    }
  } as unknown as McpServer
  registerWorkflowTools(server)
  return tools
}

const node = (nodeId: string, status: NodeExecutionState['status']): NodeExecutionState => ({
  nodeId,
  status
})

const run = (...nodeStates: NodeExecutionState[]): WorkflowExecution => ({
  runId: 'run-1',
  workflowId: 'wf-1',
  startedAt: '2026-09-05T00:00:00.000Z',
  status: 'running',
  nodeStates
})

const workflow = (message?: string): WorkflowDefinition =>
  ({
    id: 'wf-1',
    name: 'Build connector',
    nodes: [
      {
        id: 'develop',
        type: 'launchAgent',
        label: 'Develop',
        config: {},
        position: { x: 0, y: 0 }
      },
      {
        id: 'approve',
        type: 'approval',
        label: 'Approval Gate',
        config: message === undefined ? {} : { message },
        position: { x: 0, y: 0 }
      }
    ],
    edges: []
  }) as unknown as WorkflowDefinition

describe('choosing the gate a decision answers', () => {
  it('takes the one node that is waiting, since naming it would be redundant', () => {
    const target = resolveGateTarget(run(node('develop', 'success'), node('approve', 'waiting')))
    expect(target).toEqual({ nodeId: 'approve' })
  })

  it('refuses a run where nothing is waiting', () => {
    const target = resolveGateTarget(run(node('develop', 'running')))
    expect(target).toEqual({ error: 'no node in this run is waiting on a gate' })
  })

  it('asks which gate when a run has more than one open', () => {
    const target = resolveGateTarget(
      run(node('approve-a', 'waiting'), node('approve-b', 'waiting'))
    )
    expect('error' in target && target.error).toContain('pass node_id')
    expect('error' in target && target.error).toContain('approve-a, approve-b')
  })

  it('answers the named gate when a run has several', () => {
    const target = resolveGateTarget(
      run(node('approve-a', 'waiting'), node('approve-b', 'waiting')),
      'approve-b'
    )
    expect(target).toEqual({ nodeId: 'approve-b' })
  })

  it('refuses a named node that is not the one waiting, and says which is', () => {
    const target = resolveGateTarget(
      run(node('develop', 'success'), node('approve', 'waiting')),
      'develop'
    )
    expect('error' in target && target.error).toBe(
      'node "develop" is not waiting. Waiting: approve'
    )
  })
})

describe('what a gate asks', () => {
  it('reads the message the approval node was given', () => {
    expect(gateMessage(workflow('Read spec.md, then approve.'), 'approve')).toBe(
      'Read spec.md, then approve.'
    )
  })

  it('says nothing for a gate that asks nothing, or for a step that is not one', () => {
    expect(gateMessage(workflow(), 'approve')).toBeUndefined()
    expect(gateMessage(workflow('   '), 'approve')).toBeUndefined()
    expect(gateMessage(workflow('Approve?'), 'develop')).toBeUndefined()
    expect(gateMessage(undefined, 'approve')).toBeUndefined()
  })
})

describe('listing runs', () => {
  it('puts the question beside the node that is waiting on it', () => {
    const [listed] = annotateWaitingGates(
      [run(node('develop', 'success'), node('approve', 'waiting'))],
      [workflow('Read spec.md, then approve.')]
    )
    expect(listed.nodeStates.map((n) => (n as { asks?: string }).asks)).toEqual([
      undefined,
      'Read spec.md, then approve.'
    ])
  })

  it('leaves a run with nothing waiting exactly as it came', () => {
    const finished = run(node('develop', 'success'))
    const [listed] = annotateWaitingGates([finished], [workflow('Approve?')])
    expect(listed).toBe(finished)
  })
})

describe('answering a gate', () => {
  const parked = { ...run(node('develop', 'success'), node('approve', 'waiting')) }

  beforeEach(() => {
    rpcCall.mockReset().mockResolvedValue({ accepted: true })
    listAllWorkflowRuns.mockReset().mockResolvedValue([{ ...parked, workflowName: 'Build' }])
    dbListWorkflows.mockReset().mockResolvedValue([workflow('Read spec.md, then approve.')])
  })

  const resolve = async (args: Record<string, unknown>) => {
    const tool = collect().get('resolve_gate')
    if (!tool) throw new Error('resolve_gate was not registered')
    return tool(args)
  }

  it('approves the node that was waiting, and says what it answered', async () => {
    const result = await resolve({ run_id: 'run-1', decision: 'approve' })

    expect(rpcCall).toHaveBeenCalledWith('workflow:resolveGate', {
      runId: 'run-1',
      nodeId: 'approve',
      decision: 'approve'
    })
    expect(result.isError).toBeUndefined()
    expect(result.content[0].text).toContain('Approved "Approval Gate"')
    expect(result.content[0].text).toContain('Read spec.md, then approve.')
  })

  it('rejects when that is the decision', async () => {
    const result = await resolve({ run_id: 'run-1', decision: 'reject' })

    expect(rpcCall).toHaveBeenCalledWith('workflow:resolveGate', {
      runId: 'run-1',
      nodeId: 'approve',
      decision: 'reject'
    })
    expect(result.content[0].text).toContain('Rejected "Approval Gate"')
  })

  it('refuses a run that already finished, rather than broadcasting at nothing', async () => {
    listAllWorkflowRuns.mockResolvedValue([{ ...parked, status: 'success' }])

    const result = await resolve({ run_id: 'run-1', decision: 'approve' })

    expect(result.isError).toBe(true)
    expect(result.content[0].text).toContain('already finished (success)')
    expect(rpcCall).not.toHaveBeenCalled()
  })

  it('refuses a run it cannot find', async () => {
    listAllWorkflowRuns.mockResolvedValue([])

    const result = await resolve({ run_id: 'run-9', decision: 'approve' })

    expect(result.isError).toBe(true)
    expect(result.content[0].text).toContain('no run "run-9"')
    expect(rpcCall).not.toHaveBeenCalled()
  })

  it('asks which gate when a run has two open, and answers the named one', async () => {
    listAllWorkflowRuns.mockResolvedValue([run(node('a', 'waiting'), node('b', 'waiting'))])

    const asked = await resolve({ run_id: 'run-1', decision: 'approve' })
    expect(asked.isError).toBe(true)
    expect(asked.content[0].text).toContain('pass node_id')
    expect(rpcCall).not.toHaveBeenCalled()

    await resolve({ run_id: 'run-1', decision: 'approve', node_id: 'b' })
    expect(rpcCall).toHaveBeenCalledWith('workflow:resolveGate', {
      runId: 'run-1',
      nodeId: 'b',
      decision: 'approve'
    })
  })
})

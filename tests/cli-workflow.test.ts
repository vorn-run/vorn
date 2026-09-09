import { describe, it, expect, vi } from 'vitest'
import type { WorkflowDefinition, WorkflowExecution } from '../packages/shared/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('../packages/server/src/index', () => ({ startServer: vi.fn() }))

import { runCli, type CliDeps } from '../packages/server/src/cli'
import type { RpcTransport } from '../packages/server/src/cli/transport'

type Handlers = Record<string, (params: unknown) => unknown>

function fakeRpc(handlers: Handlers): {
  transport: RpcTransport
  calls: { method: string; params: unknown }[]
} {
  const calls: { method: string; params: unknown }[] = []
  const transport = {
    async call(method: string, params: unknown) {
      calls.push({ method, params })
      const handler = handlers[method]
      if (!handler) throw new Error(`unexpected call ${method}`)
      return handler(params)
    },
    async notify(method: string, params: unknown) {
      calls.push({ method, params })
    },
    isRunning: () => true
  } as unknown as RpcTransport
  return { transport, calls }
}

function capture(rpc: RpcTransport, isTty = false) {
  const out: string[] = []
  const err: string[] = []
  const deps: CliDeps & { out: () => string; err: () => string } = {
    write: (t) => out.push(t),
    writeErr: (t) => err.push(t),
    rpc,
    ensureServer: async () => true,
    isTty,
    out: () => out.join(''),
    err: () => err.join('')
  }
  return deps
}

const workflow = (over: Partial<WorkflowDefinition> = {}): WorkflowDefinition =>
  ({
    id: '7c2e11a0-1111-2222-3333-444455556666',
    name: 'Nightly review',
    icon: 'Zap',
    iconColor: '#6366f1',
    enabled: true,
    nodes: [{ id: 'n1', type: 'trigger', label: 'Trigger', config: { triggerType: 'recurring' } }],
    edges: [],
    ...over
  }) as WorkflowDefinition

const run = (over: Partial<WorkflowExecution> = {}): WorkflowExecution =>
  ({
    runId: 'r8813f01-aaaa-bbbb-cccc-ddddeeeeffff',
    workflowId: '7c2e11a0-1111-2222-3333-444455556666',
    startedAt: new Date().toISOString(),
    status: 'success',
    nodeStates: [],
    ...over
  }) as WorkflowExecution

describe('workflow list', () => {
  it('names what starts each one and whether it is on', async () => {
    const { transport } = fakeRpc({ 'workflow:list': () => [workflow()] })
    const io = capture(transport)

    expect(await runCli(['workflow', 'list'], io)).toBe(0)
    expect(io.out()).toContain('Nightly review')
    expect(io.out()).toContain('recurring')
    expect(io.out()).toContain('yes')
  })

  it('calls a workflow with no trigger node what it is', async () => {
    const { transport } = fakeRpc({ 'workflow:list': () => [workflow({ nodes: [] })] })
    const io = capture(transport)

    await runCli(['workflow', 'list'], io)
    expect(io.out()).toContain('none')
  })

  it('colours the status word for a terminal, and only that word', async () => {
    delete process.env.NO_COLOR
    const { transport } = fakeRpc({
      'workflow:list': () => [
        workflow({ lastRunAt: new Date().toISOString(), lastRunStatus: 'success' })
      ]
    })
    const io = capture(transport, true)

    await runCli(['workflow', 'list'], io)
    const escape = String.fromCharCode(27)
    expect(io.out()).toContain(`${escape}[32msuccess`)
    expect(io.out()).toContain('just now')
  })

  it('hands the definitions through untouched as json', async () => {
    const workflows = [workflow()]
    const { transport } = fakeRpc({ 'workflow:list': () => workflows })
    const io = capture(transport)

    await runCli(['workflow', 'list', '--json'], io)
    expect(JSON.parse(io.out())).toEqual(workflows)
  })
})

describe('workflow runs', () => {
  it('reads every workflow when none is named', async () => {
    const { transport, calls } = fakeRpc({
      'workflow:list': () => [workflow()],
      'workflowRun:listAll': () => [run()]
    })
    const io = capture(transport)

    expect(await runCli(['workflow', 'runs', '--limit', '5'], io)).toBe(0)
    expect(calls.at(-1)).toEqual({ method: 'workflowRun:listAll', params: { limit: 5 } })
    expect(io.out()).toContain('Nightly review')
  })

  it('takes a workflow by name and asks for that one', async () => {
    const { transport, calls } = fakeRpc({
      'workflow:list': () => [workflow()],
      'workflowRun:list': () => [run()]
    })
    const io = capture(transport)

    await runCli(['workflow', 'runs', '--workflow', 'nightly review'], io)
    expect(calls.at(-1)).toEqual({
      method: 'workflowRun:list',
      params: { workflowId: '7c2e11a0-1111-2222-3333-444455556666', limit: undefined }
    })
  })

  it('refuses an id prefix that names more than one, rather than picking one', async () => {
    const { transport } = fakeRpc({
      'workflow:list': () => [
        workflow({ id: 'import:first', name: 'Build connector' }),
        workflow({ id: 'import:second', name: 'Build connector twice' })
      ]
    })
    const io = capture(transport)

    expect(await runCli(['workflow', 'runs', '--workflow', 'import:'], io)).toBe(1)
    expect(io.err()).toContain('matches 2 workflows')
    expect(io.err()).toContain('Build connector')
  })

  it('takes a full id even when it is a prefix of another', async () => {
    const { transport, calls } = fakeRpc({
      'workflow:list': () => [
        workflow({ id: 'import:first', name: 'Build connector' }),
        workflow({ id: 'import:first-again', name: 'Build connector again' })
      ],
      'workflowRun:list': () => []
    })
    const io = capture(transport)

    expect(await runCli(['workflow', 'runs', '--workflow', 'import:first'], io)).toBe(0)
    expect(calls.at(-1)?.params).toMatchObject({ workflowId: 'import:first' })
  })

  it('says so when the name matches nothing', async () => {
    const { transport } = fakeRpc({ 'workflow:list': () => [workflow()] })
    const io = capture(transport)

    expect(await runCli(['workflow', 'runs', '--workflow', 'weekly'], io)).toBe(1)
    expect(io.err()).toContain('no workflow matches "weekly"')
  })
})

describe('workflow run', () => {
  it('starts one and answers with the run, rather than waiting for it', async () => {
    const { transport, calls } = fakeRpc({
      'workflow:list': () => [workflow()],
      'workflow:run': () => run({ status: 'running' })
    })
    const io = capture(transport)

    expect(await runCli(['workflow', 'run', 'Nightly review'], io)).toBe(0)
    expect(calls.at(-1)).toEqual({
      method: 'workflow:run',
      params: { workflowId: '7c2e11a0-1111-2222-3333-444455556666' },
      timeoutMs: undefined
    })
    expect(io.out()).toContain('run      r8813f01-aaaa-bbbb-cccc-ddddeeeeffff')
  })

  it('carries --input pairs into the run', async () => {
    const { transport, calls } = fakeRpc({
      'workflow:list': () => [workflow()],
      'workflow:run': () => run()
    })
    const io = capture(transport)

    await runCli(
      ['workflow', 'run', 'Nightly review', '--input', 'pr=42', '--input', 'branch=main'],
      io
    )
    expect(calls.at(-1)?.params).toEqual({
      workflowId: '7c2e11a0-1111-2222-3333-444455556666',
      context: { inputs: { pr: '42', branch: 'main' } }
    })
  })

  it('refuses an input that is not a pair', async () => {
    const io = capture(fakeRpc({}).transport)
    expect(await runCli(['workflow', 'run', 'x', '--input', 'nope'], io)).toBe(2)
    expect(io.err()).toContain('--input wants key=value')
  })

  it('says so when the server could not start it', async () => {
    const { transport } = fakeRpc({
      'workflow:list': () => [workflow()],
      'workflow:run': () => null
    })
    const io = capture(transport)

    expect(await runCli(['workflow', 'run', 'Nightly review'], io)).toBe(1)
    expect(io.err()).toContain('did not start')
  })
})

describe('workflow stop', () => {
  it('takes any prefix of a run id that names one', async () => {
    const { transport, calls } = fakeRpc({
      'workflowRun:listAll': () => [run()],
      'workflow:stopRun': () => undefined
    })
    const io = capture(transport)

    expect(await runCli(['workflow', 'stop', 'r8813f01'], io)).toBe(0)
    expect(calls.at(-1)).toEqual({
      method: 'workflow:stopRun',
      params: { runId: 'r8813f01-aaaa-bbbb-cccc-ddddeeeeffff' },
      timeoutMs: undefined
    })
    expect(io.out()).toBe('')
  })

  it('refuses a prefix that names more than one run', async () => {
    const { transport } = fakeRpc({
      'workflowRun:listAll': () => [run({ runId: 'aa-1' }), run({ runId: 'aa-2' })]
    })
    const io = capture(transport)

    expect(await runCli(['workflow', 'stop', 'aa'], io)).toBe(1)
    expect(io.err()).toContain('matches 2 runs')
  })
})

describe('workflow dispatch', () => {
  it('treats a bare noun as a usage error, on stderr', async () => {
    const io = capture(fakeRpc({}).transport)
    expect(await runCli(['workflow'], io)).toBe(2)
    expect(io.out()).toBe('')
    expect(io.err()).toContain('vorn workflow list')
  })

  it('reports a verb it does not have', async () => {
    const io = capture(fakeRpc({}).transport)
    expect(await runCli(['workflow', 'dance'], io)).toBe(2)
    expect(io.err()).toContain('unknown workflow command "dance"')
  })
})

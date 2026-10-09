/**
 * The work model, run by vornd, through a real server and the vornd it keeps.
 *
 * Workflows are written, run and answered through vornd; a client connected
 * to the server directly reaches the same answers, which the server hands to
 * vornd. Runs start agents and scripts through vornd's own session code, a
 * gate is answered round by round, a webhook delivered twice runs once, an
 * artifact's address answers on the server's port and vornd's, a task moved
 * on the board starts the workflow watching for it, and a schedule that fell
 * due while vornd was down fires once when it is back.
 *
 * Runs where vornd and its session holder have been built.
 */
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { WorkflowDefinition, WorkflowExecution } from '../packages/shared/src/types'
import {
  type RealServer,
  Watcher,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer,
  until
} from './helpers/real-server'

vi.setConfig({ testTimeout: 120_000, hookTimeout: 120_000 })

const pos = { x: 0, y: 0 }

function workflow(
  id: string,
  trigger: Record<string, unknown>,
  rest: Array<Record<string, unknown>>,
  edges: Array<[string, string]>
): WorkflowDefinition {
  return {
    id,
    name: id,
    icon: 'Zap',
    iconColor: '#ffffff',
    enabled: true,
    nodes: [
      { id: 't', type: 'trigger', label: 'Trigger', position: pos, config: trigger },
      ...rest
    ],
    edges: edges.map(([source, target]) => ({ id: `${source}-${target}`, source, target }))
  } as unknown as WorkflowDefinition
}

const script = (id: string, content: string): Record<string, unknown> => ({
  id,
  type: 'script',
  label: id,
  slug: id,
  position: pos,
  config: { scriptType: 'bash', scriptContent: content }
})

describe.runIf(runnable)('the work model in vornd', () => {
  let server: RealServer
  let viaVornd: Watcher
  let direct: Watcher

  const runsOf = (workflowId: string): Promise<WorkflowExecution[]> =>
    viaVornd.result<WorkflowExecution[]>('workflowRun:list', { workflowId })

  async function settled(workflowId: string, count = 1): Promise<WorkflowExecution[]> {
    let runs: WorkflowExecution[] = []
    await until(`${count} finished run(s) of ${workflowId}`, async () => {
      runs = await runsOf(workflowId)
      return runs.length >= count && runs.every((r) => r.status !== 'running')
    })
    return runs
  }

  async function connect(s: RealServer): Promise<void> {
    viaVornd = await Watcher.open(s.vornd)
    direct = await Watcher.open(s.port)
    await viaVornd.result('config:load')
    await direct.result('config:load')
  }

  beforeAll(async () => {
    server = await startRealServer()
    await connect(server)
    // The seeded workflows start real agents on a task's move; none is wanted here.
    for (const wf of await viaVornd.result<WorkflowDefinition[]>('workflow:list')) {
      await viaVornd.result('workflow:setEnabled', { id: wf.id, enabled: false })
    }
  })

  afterEach((ctx) => {
    if (process.env.DEBUG_WORK && ctx.task.result?.state === 'fail') {
      console.log(
        server.log
          .join('')
          .split('\n')
          .filter((l) => /vornd|workflow|webhook|trigger|sched/i.test(l))
          .slice(-80)
          .join('\n')
      )
    }
  })

  afterAll(async () => {
    viaVornd?.close()
    direct?.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  })

  it('answers the work model in vornd, and to a client of the server through vornd', async () => {
    const wf = workflow('wf-list', { triggerType: 'manual' }, [script('a', 'echo a')], [['t', 'a']])
    expect(await viaVornd.result('workflow:create', { workflow: wf })).toMatchObject({
      id: 'wf-list'
    })
    const listed = await direct.result<WorkflowDefinition[]>('workflow:list')
    expect(listed.map((w) => w.id)).toContain('wf-list')
    const heard = direct.toldBy('config:changed').length
    expect(await direct.result('workflow:setEnabled', { id: 'wf-list', enabled: false })).toEqual({
      ok: true
    })
    // The desktop draws the workflow's dot from the configuration it is told of.
    await until(
      'clients to be told the configuration changed',
      () => direct.toldBy('config:changed').length > heard
    )
    expect(await viaVornd.result('workflow:get', { id: 'wf-list' })).toMatchObject({
      enabled: false
    })
    expect(await viaVornd.result('workflow:setEnabled', { id: 'nope', enabled: true })).toEqual({
      ok: false
    })
    expect(await direct.result('webhook:info')).toEqual({
      baseUrl: `http://127.0.0.1:${server.port}`
    })
    expect(await viaVornd.result('workflow:delete', { id: 'wf-list' })).toEqual({ ok: true })
  })

  it('takes a gate round trip: a review page, a request for changes, then an approval', async () => {
    const wf = workflow(
      'wf-gate',
      { triggerType: 'manual' },
      [
        script('draft', 'echo "draft {{steps.review.round}}"'),
        {
          id: 'review',
          type: 'approval',
          label: 'Review',
          slug: 'review',
          position: pos,
          config: {
            message: 'Look at {{steps.draft.output}}',
            view: '<h1>{{steps.draft.output}}</h1>',
            edit: '{{steps.draft.output}}',
            feedback: { from: 'draft', maxRounds: 3 }
          }
        },
        script('ship', 'echo "shipped {{steps.review.text}}"')
      ],
      [
        ['t', 'draft'],
        ['draft', 'review'],
        ['review', 'ship']
      ]
    )
    await viaVornd.result('workflow:create', { workflow: wf })
    const started = await direct.result<WorkflowExecution>('workflow:run', {
      workflowId: 'wf-gate'
    })
    expect(started.status).toBe('running')

    const asking = async (round: number): Promise<WorkflowExecution> => {
      let run!: WorkflowExecution
      await until(`the gate to ask, round ${round}`, async () => {
        run = (await runsOf('wf-gate')).find((r) => r.runId === started.runId)!
        const gate = run?.nodeStates.find((n) => n.nodeId === 'review')
        return gate?.status === 'waiting' && gate.round === round
      })
      return run
    }

    let run = await asking(1)
    const gate = run.nodeStates.find((n) => n.nodeId === 'review')!
    expect(gate.message).toBe('Look at draft')
    expect(gate.viewToken).toMatch(/^[0-9a-f]{32}$/)
    for (const port of [server.port, server.vornd]) {
      const page = await fetch(
        `http://127.0.0.1:${port}/gate-view/${started.runId}/review?t=${gate.viewToken}`
      )
      expect(page.status).toBe(200)
      expect(page.headers.get('content-security-policy')).toContain("default-src 'none'")
      expect(await page.text()).toContain('<h1>draft')
    }
    const wrong = await fetch(
      `http://127.0.0.1:${server.port}/gate-view/${started.runId}/review?t=nope`
    )
    expect(wrong.status).toBe(404)
    expect(viaVornd.toldBy('workflow:runUpdated').length).toBeGreaterThan(0)

    expect(
      await viaVornd.result('workflow:resolveGate', {
        runId: started.runId,
        nodeId: 'review',
        decision: 'changes',
        comment: ''
      })
    ).toEqual({ accepted: false })
    expect(
      await direct.result('workflow:resolveGate', {
        runId: started.runId,
        nodeId: 'review',
        decision: 'changes',
        comment: 'again'
      })
    ).toEqual({ accepted: true })

    run = await asking(2)
    expect(run.nodeStates.find((n) => n.nodeId === 'draft')?.output).toContain('draft 2')
    expect(
      await viaVornd.result('workflow:resolveGate', {
        runId: started.runId,
        nodeId: 'review',
        decision: 'approve',
        edited: 'final words'
      })
    ).toEqual({ accepted: true })

    const [done] = (await settled('wf-gate')).filter((r) => r.runId === started.runId)
    expect(done.status).toBe('success')
    expect(done.nodeStates.find((n) => n.nodeId === 'ship')?.output).toContain(
      'shipped final words'
    )
    expect(direct.toldBy('workflow:gateResolved')).toContainEqual({
      runId: started.runId,
      nodeId: 'review',
      decision: 'approve'
    })
  })

  it('runs a webhook delivered twice once', async () => {
    const wf = workflow(
      'wf-hook',
      { triggerType: 'webhook', method: 'POST', token: 'tok-1' },
      [script('echo', 'echo "got {{trigger.body.n}}"')],
      [['t', 'echo']]
    )
    await viaVornd.result('workflow:create', { workflow: wf })
    const post = (key: string, port = server.port): Promise<Response> =>
      fetch(`http://127.0.0.1:${port}/wf-hooks/wf-hook/tok-1`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', 'idempotency-key': key },
        body: JSON.stringify({ n: 7 })
      })
    expect((await post('delivery-1')).status).toBe(202)
    expect((await post('delivery-1', server.vornd)).status).toBe(202)
    const [run] = await settled('wf-hook')
    expect(run.status).toBe('success')
    expect(run.nodeStates.find((n) => n.nodeId === 'echo')?.output).toContain('got 7')
    // Long enough for a second run to have shown.
    await new Promise((r) => setTimeout(r, 1_500))
    expect(await runsOf('wf-hook')).toHaveLength(1)

    expect((await post('delivery-2')).status).toBe(202)
    await settled('wf-hook', 2)
    const miss = await fetch(`http://127.0.0.1:${server.port}/wf-hooks/wf-hook/wrong`, {
      method: 'POST'
    })
    expect(miss.status).toBe(404)
  })

  it('serves an artifact at the address it hands out, on the server and on vornd', async () => {
    const shell = await viaVornd.result<{ id: string }>('shell:create', server.dirs.work)
    const published = await viaVornd.result<{
      url: string
      artifact: { id: string }
      version: { version: number }
      opened: boolean
    }>('artifact:publish', {
      sessionId: shell.id,
      kind: 'page',
      title: 'Figures',
      content: '<h1>Figures</h1>',
      open: false
    })
    expect(published.url.startsWith(`http://127.0.0.1:${server.port}/artifact/`)).toBe(true)
    const page = await fetch(published.url)
    expect(page.status).toBe(200)
    expect(page.headers.get('content-security-policy')).toContain("default-src 'none'")
    expect(await page.text()).toBe('<h1>Figures</h1>')
    const onVornd = await fetch(published.url.replace(`:${server.port}/`, `:${server.vornd}/`))
    expect(await onVornd.text()).toBe('<h1>Figures</h1>')
    const listed = await direct.result<Array<{ id: string }>>('artifact:list', {
      sessionId: shell.id
    })
    expect(listed.map((a) => a.id)).toContain(published.artifact.id)
    const again = await direct.result<{ url: string }>('artifact:versionUrl', {
      artifactId: published.artifact.id
    })
    expect(again.url).toBe(published.url)
    const bad = await fetch(published.url.replace(/t=[^&]+/, 't=nope'))
    expect(bad.status).toBe(404)
    await viaVornd.result('terminal:kill', shell.id)
  })

  it('starts the workflow watching a task move once, from a save on the server', async () => {
    const wf = workflow(
      'wf-task',
      { triggerType: 'taskStatusChanged', toStatus: 'in_progress' },
      [script('pick', 'echo "picked {{task.title}}"')],
      [['t', 'pick']]
    )
    await viaVornd.result('workflow:create', { workflow: wf })
    // A save carries the whole configuration: it is read once the server has seen the new workflow.
    let config!: { tasks: unknown[]; projects: unknown[]; workflows: WorkflowDefinition[] }
    await until('the server to read the new workflow', async () => {
      config = await direct.result('config:load')
      return config.workflows.some((w) => w.id === 'wf-task')
    })
    const task = {
      id: 'task-move',
      projectName: 'p',
      title: 'Moved',
      description: '',
      status: 'todo',
      order: 0,
      createdAt: '2026-01-01T00:00:00.000Z',
      updatedAt: '2026-01-01T00:00:00.000Z'
    }
    await direct.result('config:save', {
      ...config,
      projects: [{ name: 'p', path: server.dirs.work, preferredAgents: [] }],
      tasks: [task]
    })
    const before = await direct.result<Record<string, unknown>>('config:load')
    await direct.result('config:save', {
      ...before,
      tasks: [{ ...task, status: 'in_progress', updatedAt: '2026-01-01T00:01:00.000Z' }]
    })
    const [run] = await settled('wf-task')
    expect(run.triggerTaskId).toBe('task-move')
    expect(run.nodeStates.find((n) => n.nodeId === 'pick')?.output).toContain('picked Moved')
    await new Promise((r) => setTimeout(r, 1_500))
    expect(await runsOf('wf-task')).toHaveLength(1)
  })

  it('fires a schedule that fell due while vornd was down once it is back, and once only', async () => {
    const runAt = new Date(Date.now() + 6_000).toISOString()
    const wf = workflow(
      'wf-once',
      { triggerType: 'once', runAt },
      [script('tick', 'echo tick')],
      [['t', 'tick']]
    )
    await viaVornd.result('workflow:create', { workflow: wf })
    const mark = path.join(server.dirs.data, 'vornd', 'schedule.json')
    await until('the scheduler to keep how far it got', () => fs.existsSync(mark))

    viaVornd.close()
    direct.close()
    const dirs = server.dirs
    await stopRealServer(server, true)
    expect(Date.now()).toBeLessThan(new Date(runAt).getTime())
    await until(
      'the schedule to fall due while nothing runs',
      () => Date.now() > new Date(runAt).getTime() + 1_000
    )

    server = await startRealServer(dirs)
    await connect(server)
    const [run] = await settled('wf-once')
    expect(run.status).toBe('success')
    const log = await viaVornd.result<Array<{ workflowId: string }>>('scheduler:getLog', 'wf-once')
    expect(log).toHaveLength(1)

    viaVornd.close()
    direct.close()
    await stopRealServer(server, true)
    server = await startRealServer(dirs)
    await connect(server)
    await new Promise((r) => setTimeout(r, 3_000))
    expect(await runsOf('wf-once')).toHaveLength(1)
  })
})

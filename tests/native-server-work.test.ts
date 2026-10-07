/**
 * vornd's shadow of the work model: the `workflow:`, `workflowRun:`,
 * `scheduler:`, `webhook:` and `artifact:` reads, and the schedule's fires.
 *
 * One server is started on a real database with workflows, runs, a schedule
 * log and artifacts in it, and one vornd shadows those groups in front of it.
 * Every read is made directly to the server and through vornd, which forwards
 * it, computes its own answer and counts whether the two agree. A workflow
 * set to run once a few seconds ahead fires on the server; vornd sees its tick
 * lock and counts the fire it had planned as matched.
 *
 * Runs where vornd has been built (`yarn build:core`, or the binary in
 * `VORN_CONFORMANCE_VORND`).
 */
import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { WorkflowDefinition } from '../packages/shared/src/types'

const TEST_CREDENTIAL = 'native-server-work-credential'
const EXE = process.platform === 'win32' ? '.exe' : ''
const GROUPS = ['workflow', 'workflowRun', 'scheduler', 'webhook', 'artifact']

const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, `../packages/core/vornd${EXE}`),
  path.resolve(__dirname, `../packages/core/target/release/vornd${EXE}`)
].find((p): p is string => !!p && fs.existsSync(p))

// Booting a server probes Tailscale with a real process; nothing here needs it.
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

type Frame = Record<string, unknown>

/** A WebSocket client that sends one call at a time and returns its frame. */
class Client {
  private next = 1
  private constructor(private ws: WebSocket) {}

  static async open(port: number): Promise<Client> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { Authorization: `Bearer ${TEST_CREDENTIAL}` }
    })
    await new Promise<void>((resolve, reject) => {
      ws.once('open', resolve)
      ws.once('error', reject)
    })
    const client = new Client(ws)
    await client.call('config:load')
    return client
  }

  call(method: string, params?: unknown): Promise<Frame> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 30_000)
      const onMessage = (raw: WebSocket.RawData): void => {
        const frame = JSON.parse(raw.toString()) as Frame
        if (frame.id !== id) return
        this.ws.off('message', onMessage)
        clearTimeout(timer)
        resolve(frame)
      }
      this.ws.on('message', onMessage)
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  close(): void {
    this.ws.close()
  }
}

function answerOf(frame: Frame): Frame {
  const { id: _id, ...rest } = frame
  return JSON.parse(JSON.stringify(rest)) as Frame
}

interface Vornd {
  port: number
  child: ChildProcess
}

async function startVornd(upstream: number, args: string[]): Promise<Vornd> {
  const child = spawn(vornd!, ['--upstream', `127.0.0.1:${upstream}`, ...args], {
    stdio: ['ignore', 'pipe', 'inherit'],
    // The tick locks are read where the server writes them, under the same home.
    env: { ...process.env, HOME: os.homedir(), VORND_LOG: process.env.VORND_LOG ?? 'warn' }
  })
  const port = await new Promise<number>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('vornd did not start')), 10_000)
    createInterface({ input: child.stdout! }).once('line', (line) => {
      clearTimeout(timer)
      resolve((JSON.parse(line) as { port: number }).port)
    })
    child.once('exit', (code) => {
      clearTimeout(timer)
      reject(new Error(`vornd exited with ${code} before listening`))
    })
  })
  return { port, child }
}

function stopVornd(v: Vornd | undefined): Promise<void> {
  if (!v || v.child.exitCode !== null || v.child.signalCode !== null) return Promise.resolve()
  return new Promise((resolve) => {
    v.child.once('exit', () => resolve())
    v.child.kill()
  })
}

type Counts = Record<string, { mode?: string; shadowMatched?: number; shadowMismatched?: number }>

async function counts(v: Vornd): Promise<Counts> {
  const res = await fetch(`http://127.0.0.1:${v.port}/vornd/health`)
  return ((await res.json()) as { groups: Counts }).groups
}

function workflow(
  id: string,
  enabled: boolean,
  trigger: Record<string, unknown>
): WorkflowDefinition {
  return {
    id,
    name: `Workflow ${id}`,
    icon: 'Zap',
    iconColor: '#fff',
    enabled,
    edges: [],
    nodes: [
      { id: 't', type: 'trigger', label: 'Trigger', position: { x: 0, y: 0 }, config: trigger }
    ]
  } as unknown as WorkflowDefinition
}

/** A moment a few seconds ahead that is not near a minute's edge, so its tick lock and its plan share a minute. */
function soonInsideAMinute(): number {
  let at = Date.now() + 4_000
  const second = (at % 60_000) / 1000
  if (second > 54) at += (61 - second) * 1000
  return at
}

let serverPort: number
let closeServer: () => Promise<void>
let shadow: Vornd | undefined
let dataDir: string
let direct: Client
let through: Client
let ids: { doc: string; page: string; elsewhere: string }
let onceAt: number
let schedulerReads = 0

describe.skipIf(!vornd)('vornd shadows the work model as the server answers it', () => {
  beforeAll(async () => {
    process.env.SECRET_VORN_BOOTSTRAP_TOKEN = TEST_CREDENTIAL
    dataDir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-native-work-')))
    const { startServer } = await import('../packages/server/src/index')
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = (() => true) as typeof process.stdout.write
    try {
      const { app, port } = await startServer({ port: 0, dataDir })
      serverPort = port
      closeServer = () => app.close()
    } finally {
      process.stdout.write = origWrite
    }

    const db = await import('../packages/server/src/database')
    const { writeVersionBody } = await import('../packages/server/src/artifacts/bodies')
    const { publishGateArtifact } = await import('../packages/server/src/artifacts/service')
    const { configManager } = await import('../packages/server/src/config-manager')

    onceAt = soonInsideAMinute()
    for (const wf of [
      workflow('daily', true, { triggerType: 'recurring', cron: '0 9 * * 1-5' }),
      workflow('zoned', true, {
        triggerType: 'recurring',
        cron: '30 8 * * *',
        timezone: 'Asia/Tokyo'
      }),
      workflow('off', false, { triggerType: 'recurring', cron: '* * * * *' }),
      workflow('soon', true, { triggerType: 'once', runAt: new Date(onceAt).toISOString() }),
      workflow('past', true, { triggerType: 'once', runAt: '2020-01-01T00:00:00Z' }),
      workflow('manual', true, { triggerType: 'manual' })
    ]) {
      db.dbInsertWorkflow(wf)
    }
    configManager.notifyChanged()

    db.saveWorkflowRun({
      runId: 'run-1',
      workflowId: 'daily',
      startedAt: '2030-01-01T00:00:00.000Z',
      status: 'running',
      nodeStates: [],
      triggerTaskId: 'task-1'
    } as never)
    db.saveWorkflowRun({
      runId: 'run-2',
      workflowId: 'manual',
      startedAt: '2030-01-02T00:00:00.000Z',
      completedAt: '2030-01-02T00:01:00.000Z',
      status: 'success',
      nodeStates: []
    } as never)
    db.addScheduleLogEntry({
      workflowId: 'daily',
      workflowName: 'Workflow daily',
      executedAt: '2030-01-01T09:00:00.000Z',
      status: 'success',
      sessionsLaunched: 2
    })
    db.addScheduleLogEntry({
      workflowId: 'zoned',
      workflowName: 'Workflow zoned',
      executedAt: '2030-01-01T08:30:00.000Z',
      status: 'error',
      sessionsLaunched: 0,
      error: 'no agent'
    })

    const doc = db.insertArtifact({
      kind: 'doc',
      title: 'Plan',
      sessionId: null,
      projectName: 'alpha'
    }).artifact
    for (const body of ['# One', '# Two']) {
      const v = db.addArtifactVersion(doc.id, 'agent')
      writeVersionBody(dataDir, doc.id, v.version, 'doc', body)
    }
    const page = publishGateArtifact(
      dataDir,
      { runId: 'run-1', nodeId: 'gate', title: 'Review' },
      '<p>Look</p>'
    ).artifact
    const elsewhere = db.insertArtifact({
      kind: 'page',
      title: 'Bodiless',
      sessionId: null,
      projectName: 'beta'
    }).artifact
    db.addArtifactVersion(elsewhere.id, 'agent')
    ids = { doc: doc.id, page: page.id, elsewhere: elsewhere.id }

    shadow = await startVornd(serverPort, [
      '--groups',
      GROUPS.map((g) => `${g}=shadow`).join(','),
      '--db',
      path.join(dataDir, 'vorn.db')
    ])
    direct = await Client.open(serverPort)
    through = await Client.open(shadow.port)
  }, 60_000)

  afterAll(async () => {
    delete process.env.SECRET_VORN_BOOTSTRAP_TOKEN
    direct?.close()
    through?.close()
    await stopVornd(shadow)
    await closeServer?.()
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true })
  })

  it('agrees with the server on every read', async () => {
    const calls: Array<[string, unknown]> = [
      ['workflow:list', undefined],
      ['workflow:get', { id: 'daily' }],
      ['workflow:get', { id: 'missing' }],
      ['workflowRun:list', { workflowId: 'daily' }],
      ['workflowRun:list', { workflowId: 'manual', limit: 1 }],
      ['workflowRun:listByTask', { taskId: 'task-1' }],
      ['workflowRun:listByTask', { taskId: 'none', limit: 3 }],
      ['workflowRun:listWaiting', undefined],
      ['workflowRun:listRunning', undefined],
      ['workflowRun:listAll', {}],
      ['workflowRun:listAll', { limit: 1 }],
      ['scheduler:getLog', undefined],
      ['scheduler:getLog', 'zoned'],
      ['scheduler:getLog', 'missing'],
      ...['daily', 'zoned', 'off', 'soon', 'past', 'manual', 'missing'].map(
        (id): [string, unknown] => ['scheduler:getNextRun', id]
      ),
      ['webhook:info', undefined],
      ['artifact:list', {}],
      ['artifact:list', { projectName: 'alpha' }],
      ['artifact:list', { projectName: 'none', limit: 2 }],
      ['artifact:versionUrl', { artifactId: ids.doc }],
      ['artifact:versionUrl', { artifactId: ids.doc, version: 1 }],
      ['artifact:versionUrl', { artifactId: ids.doc, version: 3 }],
      ['artifact:versionUrl', { artifactId: 'missing' }],
      ['artifact:forGate', { runId: 'run-1', nodeId: 'gate' }],
      ['artifact:forGate', { runId: 'run-1', nodeId: 'other' }],
      ['artifact:readSource', { artifactId: ids.doc }],
      ['artifact:readSource', { artifactId: ids.doc, version: 1 }],
      ['artifact:readSource', { artifactId: ids.doc, version: 9 }],
      ['artifact:readSource', { artifactId: ids.page }],
      ['artifact:readSource', { artifactId: ids.elsewhere }],
      ['artifact:readSource', { artifactId: 'missing' }]
    ]
    schedulerReads = calls.filter(([method]) => method.startsWith('scheduler:')).length
    for (const [method, params] of calls) {
      const want = answerOf(await direct.call(method, params))
      expect(
        answerOf(await through.call(method, params)),
        `${method} ${JSON.stringify(params)}`
      ).toEqual(want)
    }
    const groups = await vi.waitFor(async () => {
      const now = await counts(shadow!)
      expect(now.scheduler?.shadowMatched ?? 0).toBeGreaterThanOrEqual(schedulerReads)
      for (const group of ['workflow', 'workflowRun', 'webhook', 'artifact']) {
        expect(now[group]?.shadowMatched ?? 0, `${group}`).toBeGreaterThan(0)
      }
      return now
    })
    for (const group of GROUPS) {
      expect(groups[group]?.mode, `${group}`).toBe('shadow')
      expect(groups[group]?.shadowMismatched ?? 0, `${group}`).toBe(0)
    }
  })

  it('serves the artifact URL vornd computes', async () => {
    const answer = answerOf(await through.call('artifact:versionUrl', { artifactId: ids.doc }))
    const { url } = answer.result as { url: string }
    const res = await fetch(url)
    expect(res.status).toBe(200)
    expect(await res.text()).toContain('Two')
  })

  it('plans the fire the server makes', async () => {
    const lock = path.join(
      os.homedir(),
      '.vorn',
      `scheduler-soon-${Math.floor(onceAt / 60_000)}.lock`
    )
    await vi.waitFor(() => expect(fs.existsSync(lock)).toBe(true), {
      timeout: 15_000,
      interval: 200
    })
    await vi.waitFor(
      async () =>
        expect((await counts(shadow!)).scheduler?.shadowMatched ?? 0).toBe(schedulerReads + 1),
      { timeout: 5_000, interval: 200 }
    )
    expect((await counts(shadow!)).scheduler?.shadowMismatched ?? 0).toBe(0)
  }, 30_000)
})

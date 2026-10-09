/** The `vorn` command's output and calls against a scripted server, vornd and none, as recorded in `fixtures/cli-reference.json`. */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { AddressInfo } from 'node:net'
import { WebSocketServer } from 'ws'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import {
  CliReference,
  runBinary,
  scrub,
  scrubIds,
  vornBinary,
  type Ran
} from './helpers/cli-reference'
import { builtVornd, servedDir, startServed, type Served } from './helpers/served'

const CREDENTIAL = 'native-cli-test-credential'

const reference = new CliReference()
afterAll(() => reference.save())

/** Runs `args` and checks its output against the recording under `group`; `note` tells two runs apart. */
async function pinned(
  group: string,
  args: string[],
  options: { dirs?: Record<string, string>; note?: string; ids?: boolean } = {}
): Promise<Ran> {
  const dirs = options.dirs ?? {}
  const ran = await runBinary(args)
  const clean = (text: string): string => {
    const scrubbed = posixWork(scrub(text, dirs))
    return options.ids ? scrubIds(scrubbed) : scrubbed
  }
  const got = { code: ran.code, out: clean(ran.out), err: clean(ran.err) }
  const key = [group, JSON.stringify(args.map(clean)), options.note].filter(Boolean).join(' ')
  // The key rides along, so a failure says which command it was.
  expect({ key, ...got }).toEqual({ key, ...reference.want(key, got) })
  return ran
}

/** On Windows `vorn` resolves the scripted `/work/...` paths onto its drive; reads them back as recorded. */
function posixWork(text: string): string {
  if (process.platform !== 'win32') return text
  return text.replace(
    /\b[A-Za-z]:(?:\\\\|\\)work(?![\w.-])((?:(?:\\\\|\\)[\w.-]+)*)/g,
    (_, rest: string) => `/work${rest.replace(/\\\\|\\/g, '/')}`
  )
}

function tempDir(prefix: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), prefix))
}

// ── A scripted server ───────────────────────────────────────────────────────

const LONG_AGO = '2020-01-01T00:00:00.000Z'

const TERMINALS = [
  {
    id: 'c3f1a2e8-1111-2222-3333-444455556666',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/work/vorn',
    branch: 'main',
    status: 'running'
  },
  {
    id: 'aa11bb22-1111-2222-3333-444455556666',
    agentType: 'codex',
    projectName: 'website',
    projectPath: '/work/website',
    status: 'idle'
  },
  {
    id: 'aa11cc33-1111-2222-3333-444455556666',
    agentType: 'gemini',
    projectName: 'website',
    projectPath: '/work/website',
    status: 'waiting'
  }
]

const HEADLESS = [
  {
    id: '9b4d0117-1111-2222-3333-444455556666',
    agentType: 'copilot',
    projectName: 'vorn',
    projectPath: '/work/vorn',
    status: 'running'
  },
  {
    id: '7e7e7e7e-1111-2222-3333-444455556666',
    agentType: 'opencode',
    projectName: 'vorn',
    projectPath: '/work/vorn',
    status: 'exited'
  }
]

/** A project directory as a config written on this machine holds it: on Windows, on the drive `vorn` runs from (the temp dir's). */
const here = (dir: string): string =>
  process.platform === 'win32' ? path.resolve(os.tmpdir(), dir) : dir

const CONFIG = {
  version: 1,
  defaults: { shell: '/bin/zsh', fontSize: 14, theme: 'dark' },
  projects: [
    { name: 'vorn', path: here('/work/vorn'), preferredAgents: ['claude'] },
    { name: 'website', path: here('/work/website') }
  ],
  workflows: []
}

const RECENT = [
  {
    sessionId: 'f00dfeed-1111-2222-3333-444455556666',
    agentType: 'claude',
    projectPath: '/work/website/',
    timestamp: Date.parse(LONG_AGO),
    activityLabel: '12 turns'
  },
  {
    sessionId: 'not-a-uuid',
    agentType: 'codex',
    projectPath: '/work/vorn',
    timestamp: LONG_AGO,
    activityLabel: 'idle',
    // An integer-like key, which JavaScript lists first.
    extra: { b: 1, '2': 1.5, '1': 1e21 }
  }
]

const WORKFLOWS = [
  {
    id: 'system:default-task-workflow',
    name: 'Default Task Workflow',
    enabled: true,
    nodes: [{ id: 't', type: 'trigger', config: { triggerType: 'manual' } }],
    edges: []
  },
  {
    id: '0f1e2d3c-1111-2222-3333-444455556666',
    name: 'Nightly build',
    enabled: false,
    lastRunAt: LONG_AGO,
    lastRunStatus: 'error',
    nodes: [{ id: 't', type: 'trigger', config: { triggerType: 'cron' } }],
    edges: []
  },
  {
    id: 'import:alpha',
    name: 'Twin ',
    enabled: true,
    lastRunAt: LONG_AGO,
    nodes: [{ id: 't', type: 'trigger', config: {} }],
    edges: []
  },
  { id: 'import:beta', name: 'twin', enabled: true, nodes: [], edges: [] },
  { id: 'never-starts', name: 'Never starts', enabled: true, nodes: [], edges: [] }
]

const RUNS = [
  {
    runId: '5a5a5a5a-1111-2222-3333-444455556666',
    workflowId: '0f1e2d3c-1111-2222-3333-444455556666',
    status: 'running',
    startedAt: LONG_AGO,
    nodeStates: []
  },
  {
    runId: '5a5b0000-1111-2222-3333-444455556666',
    workflowId: 'gone-workflow',
    status: 'success',
    startedAt: LONG_AGO,
    nodeStates: []
  }
]

/** One recorded call: what a side asked, in the order the server saw it. */
type Call = { method: string; params?: unknown }

interface Scripted {
  port: number
  calls: Call[]
  close(): Promise<void>
}

/** Answers a call the scripted way: a result, an error message, or a close code. */
type Script = (call: Call) => { result: unknown } | { error: string } | { close: number } | null

const script: Script = ({ method, params }) => {
  const p = (params ?? {}) as Record<string, unknown>
  switch (method) {
    case 'config:load':
      return { result: CONFIG }
    case 'config:save':
      return { result: null }
    case 'terminal:listActive':
      return { result: TERMINALS }
    case 'headless:list':
      return { result: HEADLESS }
    case 'sessions:getRecent':
      return { result: params === undefined ? RECENT : RECENT.slice(0, 1) }
    case 'terminal:readOutput':
      return {
        result: p.id === TERMINALS[0].id ? ['first line', 'second line', `lines: ${p.lines}`] : []
      }
    case 'terminal:create':
    case 'headless:create':
      return {
        result: {
          id: 'd00dd00d-1111-2222-3333-444455556666',
          status: 'running',
          ...p,
          ...(p.useWorktree ? { worktreePath: '/work/.worktrees/x' } : {}),
          ...(p.branch ? {} : { branch: null })
        }
      }
    case 'terminal:kill':
    case 'headless:kill':
    case 'workflow:stopRun':
      return { result: null }
    case 'workflow:list':
      return { result: WORKFLOWS }
    case 'workflow:run':
      return p.workflowId === 'never-starts'
        ? { result: null }
        : {
            result: {
              runId: '6b6b6b6b-1111-2222-3333-444455556666',
              workflowId: p.workflowId,
              context: p.context ?? null,
              nodeStates: [{ status: 'pending' }, { status: 'pending' }, { status: 'skipped' }]
            }
          }
    case 'workflowRun:listRunning':
      return { result: RUNS.slice(0, 1) }
    case 'workflowRun:listWaiting':
      return { result: RUNS }
    case 'workflowRun:listAll':
      return { result: p.limit === 1 ? RUNS.slice(0, 1) : RUNS }
    case 'workflowRun:list':
      return { result: RUNS.filter((r) => r.workflowId === p.workflowId) }
    default:
      return { error: `Method not found: ${method}` }
  }
}

async function scriptedServer(answer: Script = script): Promise<Scripted> {
  const calls: Call[] = []
  const wss = new WebSocketServer({
    host: '127.0.0.1',
    port: 0,
    verifyClient: (info: { req: { headers: Record<string, string | string[] | undefined> } }) =>
      info.req.headers.authorization === `Bearer ${CREDENTIAL}`
  })
  await new Promise<void>((resolve) => wss.once('listening', () => resolve()))
  wss.on('connection', (ws) => {
    ws.on('message', (raw) => {
      const frame = JSON.parse(raw.toString()) as { id?: number } & Call
      calls.push(
        frame.params === undefined
          ? { method: frame.method }
          : { method: frame.method, params: frame.params }
      )
      if (frame.id === undefined) return
      const reply = answer(frame)
      if (!reply) return
      if ('close' in reply) {
        ws.close(reply.close)
        return
      }
      // A broadcast first, which is not the answer.
      ws.send(JSON.stringify({ jsonrpc: '2.0', method: 'terminal:data', params: {} }))
      ws.send(
        JSON.stringify(
          'error' in reply
            ? { jsonrpc: '2.0', id: frame.id, error: { code: -32601, message: reply.error } }
            : { jsonrpc: '2.0', id: frame.id, result: reply.result }
        )
      )
    })
  })
  return {
    port: (wss.address() as AddressInfo).port,
    calls,
    close: () => new Promise((resolve) => wss.close(() => resolve()))
  }
}

function announce(dir: string, port: number, credential = true): void {
  fs.writeFileSync(path.join(dir, 'ws-port'), JSON.stringify({ port, pid: process.pid }))
  if (credential) fs.writeFileSync(path.join(dir, 'local-token'), `${CREDENTIAL}\n`)
}

/**
 * A notification is sent and forgotten, by both sides: it can reach the server
 * after the command has returned.
 */
function settled(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 100))
}

/** Calls are compared as a multiset: calls a side makes at once may arrive in either order. */
function sorted(calls: Call[]): string[] {
  return calls.map((c) => JSON.stringify(c)).sort()
}

describe.skipIf(!vornBinary)('vorn', () => {
  describe('without a server', () => {
    const same = (args: string[]): Promise<Ran> => pinned('no server', args)

    it('prints usage, help and version', async () => {
      expect.hasAssertions()
      await same([])
      await same(['--help'])
      await same(['-h'])
      await same(['help'])
      await same(['--json'])
      await same(['--version'])
      await same(['server'])
      await same(['server', '--help'])
      await same(['server', 'help'])
      await same(['session', '--help'])
      await same(['session'])
      await same(['workflow', '-h'])
      await same(['workflow'])
    })

    it('refuses what it does not understand', async () => {
      expect.hasAssertions()
      for (const args of [
        ['bogus'],
        ['--data-dir', 'session', 'list'],
        ['--data-dir', 'session'],
        ['--data-dir', '/tmp', 'nope'],
        ['session', 'dance'],
        ['workflow', 'dance'],
        ['session', 'list', '--nope'],
        ['session', 'list', '--json=1'],
        ['session', 'start', '--agent'],
        ['session', 'start', '--agent', '--json'],
        ['session', 'list', '-hx'],
        ['session', 'list', '--lines', '0'],
        ['session', 'list', '--limit', 'many'],
        ['session', 'list', '--timeout', '-5'],
        ['workflow', 'run', 'x', '--input', 'novalue'],
        ['workflow', 'list', '--data-dir='],
        ['session', 'list', '--version'],
        ['server', 'bogus'],
        ['server', 'serve', '--port=abc'],
        ['server', 'serve', '--nope'],
        ['serve', '--port', 'x'],
        ['token', 'create'],
        ['server', 'token', 'revoke'],
        ['token', 'wat'],
        ['token']
      ]) {
        await same(args)
      }
    })
  })

  describe('token commands, on the database file', () => {
    let dataDir: string

    beforeAll(() => {
      dataDir = tempDir('vorn-native-cli-tokens-')
    })

    afterAll(() => {
      fs.rmSync(dataDir, { recursive: true, force: true })
    })

    const tokens = (args: string[], note?: string): Promise<Ran> =>
      pinned('tokens', args, { dirs: { 'data-dir': dataDir }, note, ids: true })

    it('lists, mints, revokes and refuses', async () => {
      const dir = ['--data-dir', dataDir]
      // The first open creates and migrates the file.
      await tokens(['token', 'list', ...dir], 'empty')

      const created = await tokens(['token', 'create', '--name', 'iPhone', ...dir])
      expect(created.code).toBe(0)

      const listed = await runBinary(['token', 'list', ...dir])
      expect(listed.out).toMatch(/ {2}active {3}last seen never {2}iPhone\n$/)
      await tokens(['server', 'token', 'list', ...dir], 'one active')

      const id = listed.out.split(' ')[0]
      expect(await runBinary(['token', 'revoke', id, ...dir])).toEqual({
        code: 0,
        out: `Revoked ${id}\n`,
        err: ''
      })
      await tokens(['token', 'revoke', id, ...dir], 'again')
      await tokens(['token', 'revoke', 'not-a-token', ...dir])
      await tokens(['--data-dir', dataDir, 'token', 'list'], 'one revoked')
    }, 120_000)
  })

  describe('against a scripted server', () => {
    let server: Scripted
    let dataDir: string

    beforeAll(async () => {
      server = await scriptedServer()
      dataDir = tempDir('vorn-native-cli-scripted-')
      announce(dataDir, server.port)
    })

    afterAll(async () => {
      await server.close()
      fs.rmSync(dataDir, { recursive: true, force: true })
    })

    /** The command, and the calls it made, as recorded. */
    async function call(args: string[], dirs: Record<string, string> = {}): Promise<Ran> {
      const all = { 'data-dir': dataDir, ...dirs }
      server.calls.length = 0
      const ran = await pinned('scripted', [...args, '--data-dir', dataDir], { dirs: all })
      await settled()
      const calls = sorted(server.calls).map((c) => posixWork(scrub(c, all)))
      const key = `scripted calls ${JSON.stringify(args.map((a) => scrub(a, all)))}`
      expect({ key, calls }).toEqual({ key, calls: reference.want(key, calls) })
      return ran
    }

    it('lists sessions, as a table and as json', async () => {
      const table = await call(['session', 'list'])
      expect(table.out).toContain('ID        AGENT')
      await call(['session', 'list', '--json'])
      await call(['session', 'list', '--project', 'website'])
      await call(['session', 'list', '--project', 'nowhere'])
      await call(['session', 'list', '--recent'])
      await call(['session', 'list', '--recent', '--json'])
      await call(['session', 'list', '--recent', '--project', 'website'])
      await call(['session', 'list', '--recent', '--project', 'nope'])
      await call(['session', 'list', '--recent', '--path', '/work/./vorn/'])
    })

    it('reads, steers and kills sessions', async () => {
      expect.hasAssertions()
      await call(['session', 'logs', 'c3f1a2e8', '--lines', '50'])
      await call(['session', 'logs', 'c3f1a2e8', '--json'])
      await call(['session', 'logs', 'aa11bb22'])
      await call(['session', 'logs', 'aa11'])
      await call(['session', 'logs', 'zzz'])
      await call(['session', 'logs', '9b4d0117'])
      await call(['session', 'logs'])
      await call(['session', 'send', 'c3f1', 'hello', 'there\n'])
      await call(['session', 'send', 'c3f1', 'q', '--raw'])
      await call(['session', 'send', '9b4d', 'hi'])
      await call(['session', 'send', 'c3f1'])
      await call(['session', 'kill', '9b4d'])
      await call(['session', 'kill', 'c3f1a2e8-1111-2222-3333-444455556666'])
      await call(['session', 'kill'])
    })

    it('starts sessions', async () => {
      expect.hasAssertions()
      const parent = tempDir('vorn-native-cli-project-')
      const outside = path.join(parent, 'outside')
      fs.mkdirSync(outside)
      try {
        await call(['session', 'start'])
        await call(['session', 'start', '--agent', 'hal'])
        await call(['session', 'start', '--agent', 'claude', '--project', 'website'])
        await call(['session', 'start', '--agent', 'codex', '--path', '/work/vorn/'])
        await call(
          [
            'session',
            'start',
            '--agent',
            'gemini',
            '--path',
            outside,
            '--prompt',
            'fix it',
            '--branch',
            'feature',
            '--worktree',
            '--name',
            'Fixer'
          ],
          { project: outside }
        )
        await call(['session', 'start', '--agent', 'claude', '--headless', '--json'])
        await call(['session', 'start', '--agent', 'opencode', '--project', 'brand-new'])
      } finally {
        fs.rmSync(parent, { recursive: true, force: true })
      }
    })

    it('lists, runs and stops workflows', async () => {
      const listed = await call(['workflow', 'list'])
      expect(listed.out).toContain('system:default-task-workflow')
      await call(['workflow', 'list', '--json'])
      await call(['workflow', 'run', 'nightly BUILD', '--input', 'pr=42', '--input', 'x=a=b'])
      await call(['workflow', 'run', '0f1e', '--json'])
      await call(['workflow', 'run', 'import:'])
      await call(['workflow', 'run', 'twin'])
      await call(['workflow', 'run', 'missing'])
      await call(['workflow', 'run', 'never-starts'])
      await call(['workflow', 'run'])
      await call(['workflow', 'stop', '5a5'])
      await call(['workflow', 'stop', '5a5a'])
      await call(['workflow', 'stop', 'zz'])
      await call(['workflow', 'stop'])
      await call(['workflow', 'runs'])
      await call(['workflow', 'runs', '--limit', '1', '--json'])
      await call(['workflow', 'runs', '--workflow', 'nightly build'])
      await call(['workflow', 'runs', '--workflow', 'Never starts'])
      await call(['workflow', 'runs', '--workflow', 'import:'])
    })

    it('reports what the server could not do', async () => {
      expect.hasAssertions()
      const failing = await scriptedServer(({ method }) => ({
        error: method === 'workflow:list' ? 'Method not found: workflow:list' : 'database is locked'
      }))
      const dir = tempDir('vorn-native-cli-failing-')
      announce(dir, failing.port)
      const same = (args: string[]): Promise<Ran> =>
        pinned('failing', args, { dirs: { 'data-dir': dir } })
      try {
        await same(['workflow', 'list', '--data-dir', dir])
        await same(['session', 'list', '--data-dir', dir])
        await same(['session', 'kill', 'x', '--data-dir', dir])
      } finally {
        await failing.close()
        fs.rmSync(dir, { recursive: true, force: true })
      }
    })
  })

  describe('when the server cannot answer', () => {
    it('names a refused credential, and any other close', async () => {
      expect.hasAssertions()
      for (const code of [4001, 4002, 4000]) {
        const closing = await scriptedServer(() => ({ close: code }))
        const dir = tempDir('vorn-native-cli-closing-')
        announce(dir, closing.port)
        try {
          await pinned('closing', ['workflow', 'list', '--data-dir', dir], {
            dirs: { 'data-dir': dir },
            note: `with ${code}`
          })
        } finally {
          await closing.close()
          fs.rmSync(dir, { recursive: true, force: true })
        }
      }
    })

    it('gives up after --timeout', async () => {
      const silent = await scriptedServer(() => null)
      const dir = tempDir('vorn-native-cli-silent-')
      announce(dir, silent.port)
      try {
        const ran = await pinned(
          'silent',
          ['session', 'list', '--timeout', '300', '--data-dir', dir],
          {
            dirs: { 'data-dir': dir }
          }
        )
        expect(ran.err).toContain('timed out after 300ms')
      } finally {
        await silent.close()
        fs.rmSync(dir, { recursive: true, force: true })
      }
    })

    it('says the credential is missing', async () => {
      const dir = tempDir('vorn-native-cli-nocred-')
      announce(dir, 9, false)
      try {
        const ran = await pinned('no credential', ['workflow', 'runs', '--data-dir', dir], {
          dirs: { 'data-dir': dir }
        })
        expect(ran.err).toContain('Vorn local credential not found')
      } finally {
        fs.rmSync(dir, { recursive: true, force: true })
      }
    })

    it('says nothing listens on the announced port', async () => {
      const dir = tempDir('vorn-native-cli-refused-')
      const probe = await scriptedServer()
      const port = probe.port
      await probe.close()
      announce(dir, port)
      try {
        const ran = await pinned('refused', ['workflow', 'list', '--data-dir', dir], {
          dirs: { 'data-dir': dir }
        })
        expect(ran.err).toContain('ECONNREFUSED')
      } finally {
        fs.rmSync(dir, { recursive: true, force: true })
      }
    })
  })

  describe.skipIf(!builtVornd)('against a real server', () => {
    let dataDir: string
    let served: Served | undefined

    beforeAll(async () => {
      dataDir = servedDir('vorn-native-cli-server-')
      served = await startServed({ dataDir, credential: CREDENTIAL, sessiond: true })
    }, 30_000)

    afterAll(async () => {
      await served?.stop()
      if (served) fs.rmSync(served.home, { recursive: true, force: true })
      try {
        fs.rmSync(dataDir, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 })
      } catch (err) {
        // The session holder the server started outlives it, and Windows
        // will not delete a running program; what is left is the OS's to clear.
        if (process.platform !== 'win32') throw err
      }
    }, 30_000)

    it('answers the same commands', async () => {
      const dir = ['--data-dir', dataDir]
      const same = (args: string[]): Promise<Ran> =>
        pinned('real server', args, { dirs: { 'data-dir': dataDir } })
      expect(fs.existsSync(path.join(dataDir, 'local-token'))).toBe(true)
      const listed = await same(['workflow', 'list', ...dir])
      expect(listed.code).toBe(0)
      await same(['workflow', 'list', '--json', ...dir])
      await same(['workflow', 'runs', ...dir])
      await same(['workflow', 'runs', '--json', '--limit', '5', ...dir])
      await same(['session', 'list', ...dir])
      await same(['session', 'list', '--json', ...dir])
      await same(['session', 'list', '--recent', '--json', ...dir])
      await same(['session', 'logs', 'nope', ...dir])
      await same(['session', 'kill', 'nope', ...dir])
      await same(['workflow', 'stop', 'nope', ...dir])
      await same(['workflow', 'run', 'no such workflow', ...dir])
    })
  })
})

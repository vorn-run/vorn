/**
 * The Rust `vorn` against the TypeScript `runCli`: the same command line, the
 * same server, the same data directory, and the same stdout, stderr and exit
 * code but for the differences `helpers/cli-parity` names.
 *
 * Three servers: a scripted one, which answers every call with fixed data so
 * every table, message and error path can be compared exactly (and records
 * what each side asked, which must match too); vornd as the server, on a
 * data directory of its own; and none, for the token commands, which
 * work on the database file directly.
 *
 * Runs where the binary has been built (`yarn build:core`, cargo, or the
 * binary in `VORN_CLI_BINARY`).
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { AddressInfo } from 'node:net'
import { WebSocketServer } from 'ws'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import {
  MCP_USAGE_LINE,
  MINTED_TOKEN,
  SERVER_LOG_LINES,
  VERSION,
  normalized,
  runBinary,
  runTypeScript,
  vornBinary,
  type Ran
} from './helpers/cli-parity'

import { runCli } from '../packages/server/src/cli'
import { builtVornd, startServed, type Served } from './helpers/served'

const CREDENTIAL = 'native-cli-test-credential'

/** The environment both sides run in: no colour asked for or against, no data dir named. */
function cleanEnv(): NodeJS.ProcessEnv {
  const env = { ...process.env }
  delete env.NO_COLOR
  delete env.VORN_DATA_DIR
  return env
}

async function viaTypeScript(args: string[]): Promise<Ran> {
  const out: string[] = []
  const err: string[] = []
  const code = await runCli(args, {
    write: (text) => out.push(text),
    writeErr: (text) => err.push(text),
    isTty: false
  })
  return { code, out: out.join(''), err: err.join('') }
}

/** Runs `args` through both, and returns both with the accepted differences taken out. */
async function both(args: string[], accepted: string[] = []): Promise<{ ts: Ran; rs: Ran }> {
  const saved = { NO_COLOR: process.env.NO_COLOR, VORN_DATA_DIR: process.env.VORN_DATA_DIR }
  delete process.env.NO_COLOR
  delete process.env.VORN_DATA_DIR
  let ts: Ran
  try {
    ts = await viaTypeScript(args)
  } finally {
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
  }
  const rs = await runBinary(args, cleanEnv())
  return { ts: normalized(ts, accepted), rs: normalized(rs, accepted) }
}

async function same(args: string[], accepted: string[] = []): Promise<Ran> {
  const { ts, rs } = await both(args, accepted)
  expect(rs).toEqual(ts)
  return ts
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

const CONFIG = {
  version: 1,
  defaults: { shell: '/bin/zsh', fontSize: 14, theme: 'dark' },
  projects: [
    { name: 'vorn', path: '/work/vorn', preferredAgents: ['claude'] },
    { name: 'website', path: '/work/website' }
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

describe.skipIf(!vornBinary)('vorn, in Rust, against runCli', () => {
  describe('without a server', () => {
    it('prints usage, help and version alike', async () => {
      expect.hasAssertions()
      await same([], [MCP_USAGE_LINE])
      await same(['--help'], [MCP_USAGE_LINE])
      await same(['-h'], [MCP_USAGE_LINE])
      await same(['help'], [MCP_USAGE_LINE])
      await same(['--json'], [MCP_USAGE_LINE])
      await same(['--version'], [VERSION])
      await same(['server'])
      await same(['server', '--help'])
      await same(['server', 'help'])
      await same(['session', '--help'])
      await same(['session'])
      await same(['workflow', '-h'])
      await same(['workflow'])
    })

    it('refuses what it does not understand alike', async () => {
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

    /** Both sides as processes of their own, the TypeScript first. */
    async function tokens(args: string[], accepted: string[] = []): Promise<Ran> {
      const names = [SERVER_LOG_LINES, ...accepted]
      const ts = normalized(await runTypeScript(args, cleanEnv()), names)
      const rs = normalized(await runBinary(args, cleanEnv()), names)
      expect(rs).toEqual(ts)
      return ts
    }

    it('lists, mints, revokes and refuses alike', async () => {
      const dir = ['--data-dir', dataDir]
      // The first open creates and migrates the file; the TypeScript goes first.
      await tokens(['token', 'list', ...dir])

      const created = await tokens(['token', 'create', '--name', 'iPhone', ...dir], [MINTED_TOKEN])
      expect(created.code).toBe(0)

      const listed = await runBinary(['token', 'list', ...dir], cleanEnv())
      const [first, second] = listed.out.trim().split('\n')
      expect(first).toMatch(/ {2}active {3}last seen never {2}iPhone$/)
      expect(second).toMatch(/ {2}active {3}last seen never {2}iPhone$/)
      await tokens(['server', 'token', 'list', ...dir])

      // Each side revokes one, then neither can revoke it again.
      const ids = [first.split(' ')[0], second.split(' ')[0]]
      const tsRevoke = await runTypeScript(['token', 'revoke', ids[0], ...dir], cleanEnv())
      const rsRevoke = await runBinary(['token', 'revoke', ids[1], ...dir], cleanEnv())
      const names = [SERVER_LOG_LINES, MINTED_TOKEN]
      expect(normalized(rsRevoke, names)).toEqual(normalized(tsRevoke, names))
      expect(rsRevoke).toEqual({ code: 0, out: `Revoked ${ids[1]}\n`, err: '' })
      await tokens(['token', 'revoke', ids[0], ...dir])
      await tokens(['token', 'revoke', 'not-a-token', ...dir])
      await tokens(['--data-dir', dataDir, 'token', 'list'])
      // Each TypeScript command is a process of its own, loaded from source.
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

    /** Both sides, and the calls each made, which must be the same calls. */
    async function call(args: string[]): Promise<Ran> {
      const full = [...args, '--data-dir', dataDir]
      server.calls.length = 0
      const ts = await viaTypeScript(full)
      await settled()
      const tsCalls = sorted(server.calls)
      server.calls.length = 0
      const rs = await runBinary(full, cleanEnv())
      await settled()
      expect(sorted(server.calls)).toEqual(tsCalls)
      expect(rs).toEqual(ts)
      return ts
    }

    it('lists sessions alike, as a table and as json', async () => {
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

    it('reads, steers and kills sessions alike', async () => {
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

    it('starts sessions alike', async () => {
      expect.hasAssertions()
      const outside = tempDir('vorn-native-cli-project-')
      try {
        await call(['session', 'start'])
        await call(['session', 'start', '--agent', 'hal'])
        await call(['session', 'start', '--agent', 'claude', '--project', 'website'])
        await call(['session', 'start', '--agent', 'codex', '--path', '/work/vorn/'])
        await call([
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
        ])
        await call(['session', 'start', '--agent', 'claude', '--headless', '--json'])
        await call(['session', 'start', '--agent', 'opencode', '--project', 'brand-new'])
      } finally {
        fs.rmSync(outside, { recursive: true, force: true })
      }
    })

    it('lists, runs and stops workflows alike', async () => {
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

    it('reports what the server could not do alike', async () => {
      expect.hasAssertions()
      const failing = await scriptedServer(({ method }) => ({
        error: method === 'workflow:list' ? 'Method not found: workflow:list' : 'database is locked'
      }))
      const dir = tempDir('vorn-native-cli-failing-')
      announce(dir, failing.port)
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
    it('names a refused credential, and any other close, alike', async () => {
      expect.hasAssertions()
      for (const code of [4001, 4002, 4000]) {
        const closing = await scriptedServer(() => ({ close: code }))
        const dir = tempDir('vorn-native-cli-closing-')
        announce(dir, closing.port)
        try {
          await same(['workflow', 'list', '--data-dir', dir])
        } finally {
          await closing.close()
          fs.rmSync(dir, { recursive: true, force: true })
        }
      }
    })

    it('gives up after --timeout alike', async () => {
      const silent = await scriptedServer(() => null)
      const dir = tempDir('vorn-native-cli-silent-')
      announce(dir, silent.port)
      try {
        const ran = await same(['session', 'list', '--timeout', '300', '--data-dir', dir])
        expect(ran.err).toContain('timed out after 300ms')
      } finally {
        await silent.close()
        fs.rmSync(dir, { recursive: true, force: true })
      }
    })

    it('says the credential is missing alike', async () => {
      const dir = tempDir('vorn-native-cli-nocred-')
      announce(dir, 9, false)
      try {
        const ran = await same(['workflow', 'runs', '--data-dir', dir])
        expect(ran.err).toContain('Vorn local credential not found')
      } finally {
        fs.rmSync(dir, { recursive: true, force: true })
      }
    })

    it('says nothing listens on the announced port alike', async () => {
      const dir = tempDir('vorn-native-cli-refused-')
      const probe = await scriptedServer()
      const port = probe.port
      await probe.close()
      announce(dir, port)
      try {
        const ran = await same(['workflow', 'list', '--data-dir', dir])
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
      dataDir = tempDir('vorn-native-cli-server-')
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

    it('answers the same commands alike', async () => {
      const dir = ['--data-dir', dataDir]
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

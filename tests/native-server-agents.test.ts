/**
 * vornd's own answers to the `agent:`, `sessions:` and `shell:` calls it
 * takes over, against the server's answers to the same calls.
 *
 * The test gives both one home directory, holding a history for each of the
 * five agents, and one PATH, holding a stand-in CLI for each agent that lists
 * models. The login shell both ask for its environment is a stand-in too,
 * which prints the environment it was given, so both see that PATH. One
 * server is started, and two vornds in front of it: one answering
 * them itself, one shadowing the same groups. Every call is made to the
 * server and through vornd, and the two frames a client receives must be the
 * same but for the differences `helpers/agents-parity` names.
 *
 * Runs where vornd has been built (`yarn build:core`, or the binary in
 * `VORN_CONFORMANCE_VORND`).
 */
import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'
import Database from 'libsql'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { answerOf, type Answer } from './helpers/git-parity'
import { catalogFetchedAt } from './helpers/agents-parity'

const TEST_CREDENTIAL = 'native-server-agents-credential'
const EXE = process.platform === 'win32' ? '.exe' : ''

const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, `../packages/core/vornd${EXE}`),
  path.resolve(__dirname, `../packages/core/target/release/vornd${EXE}`)
].find((p): p is string => !!p && fs.existsSync(p))

/** The stand-in CLIs and the shell are scripts; Windows runs neither. */
const runnable = !!vornd && process.platform !== 'win32'

/** Shared with the database stub, which is hoisted above everything else. */
const shared = vi.hoisted(() => ({ dataDir: '/tmp/vorn-native-server-agents' }))

/** What the server's stub and vornd's database file both say. */
const AGENT_COMMANDS = {
  gemini: { command: 'gemini-missing', args: [], fallbackCommand: 'gem-fallback' },
  codex: { command: 'codex', args: ['--profile', 'work', '--yolo'] }
}

vi.mock('node-pty', () => ({
  default: { spawn: vi.fn() },
  spawn: vi.fn()
}))

// Booting a server probes Tailscale with a real process; nothing here needs it.
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

vi.mock(
  '../packages/server/src/database',
  () =>
    ({
      closeDatabase: vi.fn(),
      initDatabase: vi.fn(),
      getDataDir: vi.fn(() => shared.dataDir),
      dbGetOwnerUser: vi.fn(() => ({
        id: 'owner-1',
        name: 'test',
        role: 'owner' as const,
        createdAt: new Date().toISOString()
      })),
      dbInsertDeviceToken: vi.fn(),
      dbListDeviceTokens: vi.fn(() => []),
      dbGetDeviceTokenSecret: vi.fn(),
      dbRevokeDeviceToken: vi.fn(() => true),
      dbTouchDeviceToken: vi.fn(),
      loadConfig: vi.fn(() => ({
        version: 1,
        defaults: {
          shell: process.env.SHELL,
          fontSize: 14,
          theme: 'dark',
          envPassthrough: ['PARITY_SECRET_KEY']
        },
        projects: [],
        agentCommands: {
          gemini: { command: 'gemini-missing', args: [], fallbackCommand: 'gem-fallback' },
          codex: { command: 'codex', args: ['--profile', 'work', '--yolo'] }
        },
        workflows: [],
        remoteHosts: [],
        tasks: [],
        workspaces: []
      })),
      saveConfig: vi.fn(),
      dbListTasks: vi.fn(() => []),
      dbGetTask: vi.fn(),
      dbInsertTask: vi.fn(),
      dbUpdateTask: vi.fn(),
      dbDeleteTask: vi.fn(),
      dbGetMaxTaskOrder: vi.fn(() => 0),
      dbGetProject: vi.fn(),
      dbListProjects: vi.fn(() => []),
      dbListWorkflows: vi.fn(() => []),
      dbInsertWorkflow: vi.fn(),
      dbUpdateWorkflow: vi.fn(),
      dbDeleteWorkflow: vi.fn(),
      saveWorkflowRun: vi.fn(),
      listWorkflowRuns: vi.fn(() => []),
      listWorkflowRunIds: vi.fn(() => []),
      deleteArtifactsUpdatedBefore: vi.fn(() => []),
      listArtifactIds: vi.fn(() => []),
      listWorkflowRunsByTask: vi.fn(() => []),
      updateWorkflowRunStatus: vi.fn(),
      dbReleaseConnectorInboxLeases: vi.fn(),
      dbCountActiveConnectorInboxLeases: vi.fn(() => 0),
      dbClaimConnectorInbox: vi.fn(() => []),
      dbGetWorkflowRunByConnectorInboxId: vi.fn(() => null)
    }) satisfies Partial<Record<keyof typeof import('../packages/server/src/database'), unknown>>
)

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
    return new Client(ws)
  }

  call(method: string, params?: unknown): Promise<Record<string, unknown>> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 30_000)
      const onMessage = (raw: WebSocket.RawData): void => {
        const frame = JSON.parse(raw.toString()) as Record<string, unknown>
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

interface Vornd {
  port: number
  child: ChildProcess
}

async function startVornd(upstream: number, args: string[]): Promise<Vornd> {
  const child = spawn(vornd!, ['--upstream', `127.0.0.1:${upstream}`, ...args], {
    stdio: ['ignore', 'pipe', 'inherit'],
    env: { ...process.env, VORND_LOG: process.env.VORND_LOG ?? 'warn' }
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

type Counts = Record<
  string,
  {
    mode?: string
    forwarded?: number
    native?: number
    shadowMatched?: number
    shadowMismatched?: number
    shadowUnported?: number
  }
>

async function counts(v: Vornd): Promise<Counts> {
  const res = await fetch(`http://127.0.0.1:${v.port}/vornd/health`)
  return ((await res.json()) as { groups: Counts }).groups
}

function write(file: string, text: string, mode?: number): void {
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(file, text, mode === undefined ? undefined : { mode })
}

function sqlite(file: string, sql: string): void {
  fs.mkdirSync(path.dirname(file), { recursive: true })
  const db = new Database(file)
  db.exec(sql)
  db.close()
}

/**
 * A stand-in for each agent CLI that lists models, speaking just enough of
 * its protocol. In a directory holding `.fail-models` it reads the first
 * message and exits with 1, as a CLI that is not signed in would.
 */
const FAKE_CLI = `#!/usr/bin/env node
const fs = require('fs')
const path = require('path')
const readline = require('readline')
const name = path.basename(process.argv[1])
const fail = fs.existsSync('.fail-models')
if (name === 'opencode') {
  if (fail || process.argv[2] !== 'models') process.exit(1)
  process.stdout.write('Models:\\nanthropic/claude-x\\nopenai/gpt-y\\nanthropic/claude-x\\n')
  process.exit(0)
}
const send = (m) => process.stdout.write(JSON.stringify(m) + '\\n')
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  if (fail) process.exit(1)
  const msg = JSON.parse(line)
  if (name === 'claude' && msg.type === 'control_request') {
    send({ type: 'system', subtype: 'init' })
    send({ type: 'control_response', response: { subtype: 'success', request_id: msg.request_id,
      response: { models: [{ value: 'opus', displayName: 'Opus', description: 'Most capable' },
        { value: 'haiku', displayName: 'Haiku' }, { value: 'hidden', hidden: true }] } } })
  } else if (name === 'codex' && msg.method === 'initialize') {
    send({ id: msg.id, result: {} })
  } else if (name === 'codex' && msg.method === 'model/list') {
    if (msg.params.cursor) send({ id: msg.id, result: { data: [{ model: 'gpt-mini' }], nextCursor: null } })
    else send({ id: msg.id, result: { data: [{ model: 'gpt', displayName: 'GPT' }], nextCursor: 'p2' } })
  } else if (name === 'copilot' && msg.method === 'initialize') {
    send({ jsonrpc: '2.0', id: msg.id, result: {} })
  } else if (name === 'copilot' && msg.method === 'session/new') {
    send({ jsonrpc: '2.0', id: msg.id, result: { models: { availableModels: [
      { modelId: 'auto', name: 'Auto', _meta: { copilotUsage: '1x', copilotEnablement: 'enabled' } },
      { modelId: 'blocked', name: 'Blocked', _meta: { copilotEnablement: 'disabled' } }] } } })
  }
})
`

interface Fixture {
  root: string
  home: string
  project: string
  worktree: string
  linked: string
  plain: string
}

/** A home with every agent's history, and a project with a worktree. */
function makeFixture(): Fixture {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-native-agents-')))
  const home = path.join(root, 'home')
  const project = path.join(root, 'project')
  const worktree = path.join(root, '.vorn-worktrees', 'project', 'wt')
  const linked = path.join(root, 'linked')
  const plain = path.join(root, 'plain')
  fs.mkdirSync(project, { recursive: true })
  fs.mkdirSync(plain)
  const git = (...args: string[]): void => {
    execFileSync('git', args, { cwd: project, stdio: 'ignore' })
  }
  git('init', '-q', '-b', 'main')
  git(
    '-c',
    'user.email=p@vorn.invalid',
    '-c',
    'user.name=p',
    'commit',
    '-q',
    '--allow-empty',
    '-m',
    'base'
  )
  git('worktree', 'add', '-q', '-b', 'wt', worktree)
  fs.symlinkSync(project, linked)

  const claude = [
    { sessionId: 'c1', display: 'Fix the bug', project, timestamp: 1_735_787_045_000 },
    {
      sessionId: 'c2',
      display: 'In the worktree',
      project: worktree,
      timestamp: 1_735_787_046_000
    },
    { sessionId: 'c1', display: 'later prompt', project, timestamp: 1_735_787_047_000 },
    {
      sessionId: 'c3',
      display: 'Through the link',
      project: linked + '/',
      timestamp: 1_735_787_000_000
    },
    { sessionId: 'c4', display: 'Elsewhere', project: plain, timestamp: 1_735_787_048_000 },
    {
      sessionId: 'c5',
      display: 'Same time as a codex thread',
      project,
      timestamp: 1_735_700_000_000
    }
  ]
  write(
    path.join(home, '.claude', 'history.jsonl'),
    claude.map((e) => JSON.stringify(e)).join('\n') + '\nnot json\n'
  )

  const chats = path.join(home, '.gemini', 'tmp', 'abc', 'chats')
  write(path.join(home, '.gemini', 'tmp', 'abc', '.project_root'), project + '\n')
  write(
    path.join(chats, 'session-1.json'),
    JSON.stringify({
      sessionId: 'g1',
      startTime: '2025-01-01T00:00:00.000Z',
      lastUpdated: '2025-01-02T03:04:10.500Z',
      messages: [
        { id: '1', timestamp: '', type: 'user', content: [{ text: 'héllo' }, { text: 'gemini' }] },
        { id: '2', timestamp: '', type: 'gemini', content: 'hi' },
        { id: '3', timestamp: '', type: 'user', content: 'x'.repeat(200) }
      ]
    })
  )
  write(path.join(chats, 'session-2.json'), '{broken')

  sqlite(
    path.join(home, '.codex', 'state_5.sqlite'),
    `CREATE TABLE threads (id TEXT, cwd TEXT, title TEXT, updated_at INTEGER, first_user_message TEXT, archived INTEGER);
     INSERT INTO threads VALUES ('x1', '${project}', '', 1735700000, 'A first message', 0);
     INSERT INTO threads VALUES ('x2', '${plain}', 'Plain', 1735787049, NULL, 0);
     INSERT INTO threads VALUES ('x3', '${project}', 'Archived', 1735787050, NULL, 1);`
  )
  write(
    path.join(home, '.codex', 'history.jsonl'),
    '{"session_id":"x1"}\n{"session_id":"x1"}\n{"session_id":"x2"}\n'
  )
  sqlite(
    path.join(home, '.copilot', 'session-store.db'),
    `CREATE TABLE sessions (id TEXT, cwd TEXT, summary TEXT, updated_at TEXT);
     CREATE TABLE turns (id INTEGER, session_id TEXT);
     INSERT INTO sessions VALUES ('p1', '${worktree}', 'Copilot work', '2025-01-02T03:04:20Z');
     INSERT INTO turns VALUES (1, 'p1'), (2, 'p1'), (3, 'p1');`
  )
  sqlite(
    path.join(home, '.local', 'share', 'opencode', 'opencode.db'),
    `CREATE TABLE session (id TEXT, directory TEXT, title TEXT, time_updated INTEGER, time_archived INTEGER);
     CREATE TABLE message (id TEXT, session_id TEXT);
     INSERT INTO session VALUES ('o1', '${project}', 'OpenCode work', 1735787044000, NULL);
     INSERT INTO message VALUES ('m1', 'o1'), ('m2', 'o1');`
  )
  return { root, home, project, worktree, linked, plain }
}

/** The stand-in CLIs, the gemini fallback and the stand-in login shell. */
function makeTools(root: string): { bin: string; shell: string } {
  const bin = path.join(root, 'bin')
  for (const name of ['claude', 'codex', 'copilot', 'opencode']) {
    write(path.join(bin, name), FAKE_CLI, 0o755)
  }
  write(path.join(bin, 'gem-fallback'), '#!/bin/sh\n', 0o755)
  const shell = path.join(root, 'login-shell')
  write(shell, '#!/bin/sh\nexec env\n', 0o755)
  return { bin, shell }
}

/** The settings vornd reads: the same agent commands and passthrough as the stub. */
function writeStore(file: string): void {
  const db = new Database(file)
  db.exec(`
    CREATE TABLE agent_commands (agent_type TEXT PRIMARY KEY, command TEXT NOT NULL, args TEXT NOT NULL,
      headless_args TEXT, fallback_command TEXT, fallback_args TEXT, row_revision INTEGER);
    CREATE TABLE defaults (key TEXT PRIMARY KEY, value TEXT NOT NULL);
  `)
  const insert = db.prepare(
    'INSERT INTO agent_commands (agent_type, command, args, fallback_command) VALUES (?, ?, ?, ?)'
  )
  for (const [agent, cmd] of Object.entries(AGENT_COMMANDS)) {
    insert.run(
      agent,
      cmd.command,
      JSON.stringify(cmd.args),
      'fallbackCommand' in cmd ? cmd.fallbackCommand : null
    )
  }
  db.prepare('INSERT INTO defaults (key, value) VALUES (?, ?)').run(
    'envPassthrough',
    JSON.stringify(['PARITY_SECRET_KEY'])
  )
  db.close()
}

const ENV_KEYS = ['HOME', 'XDG_DATA_HOME', 'PATH', 'SHELL'] as const

/**
 * Both sides read each shell's `--version`, giving PowerShell six seconds.
 * A cold PowerShell on a CI runner can take longer, and then only the side
 * that asked first goes without a version; one warm start first keeps the
 * two answers about the machine, not about which side asked first.
 */
function warmPowerShell(): void {
  for (const name of ['pwsh', 'powershell']) {
    try {
      execFileSync(name, ['--version'], { stdio: 'ignore', timeout: 30_000 })
    } catch {
      // Not installed, which both sides see alike.
    }
  }
}

let serverPort: number
let closeServer: () => Promise<void>
let native: Vornd | undefined
let shadow: Vornd | undefined
let fx: Fixture
let savedEnv: Partial<Record<(typeof ENV_KEYS)[number], string>>

describe.skipIf(!runnable)(
  'the native server answers agent and session lookups as the server does',
  () => {
    beforeAll(async () => {
      savedEnv = Object.fromEntries(ENV_KEYS.map((k) => [k, process.env[k]]))
      fx = makeFixture()
      const tools = makeTools(fx.root)
      shared.dataDir = path.join(fx.root, 'data')
      fs.mkdirSync(shared.dataDir)
      writeStore(path.join(shared.dataDir, 'vorn.db'))
      process.env.HOME = fx.home
      process.env.XDG_DATA_HOME = path.join(fx.home, '.local', 'share')
      process.env.PATH = `${tools.bin}${path.delimiter}${process.env.PATH ?? ''}`
      process.env.SHELL = tools.shell
      warmPowerShell()

      process.env.SECRET_VORN_BOOTSTRAP_TOKEN = TEST_CREDENTIAL
      const { startServer } = await import('../packages/server/src/index')
      const { shellEnvSettled } = await import('../packages/server/src/process-utils')
      const origWrite = process.stdout.write.bind(process.stdout)
      process.stdout.write = (() => true) as typeof process.stdout.write
      try {
        const { app, port } = await startServer({ port: 0 })
        serverPort = port
        closeServer = () => app.close()
      } finally {
        process.stdout.write = origWrite
      }
      await shellEnvSettled(10_000)
      const db = path.join(shared.dataDir, 'vorn.db')
      native = await startVornd(serverPort, ['--db', db])
      shadow = await startVornd(serverPort, [
        '--groups',
        'agent=shadow,sessions=shadow,shell=shadow',
        '--db',
        db
      ])
    }, 60_000)

    afterAll(async () => {
      delete process.env.SECRET_VORN_BOOTSTRAP_TOKEN
      await Promise.all([stopVornd(native), stopVornd(shadow)])
      await closeServer?.()
      for (const key of ENV_KEYS) {
        if (savedEnv[key] === undefined) delete process.env[key]
        else process.env[key] = savedEnv[key]
      }
      if (fx?.root) fs.rmSync(fx.root, { recursive: true, force: true })
    })

    /** Calls that change nothing and start nothing. */
    const readCalls = (): Array<[string, unknown]> => [
      ['agent:detectInstalled', undefined],
      ['sessions:getRecent', undefined],
      ['sessions:getRecent', ''],
      ['sessions:getRecent', fx.project],
      ['sessions:getRecent', fx.project + '/'],
      ['sessions:getRecent', fx.worktree],
      ['sessions:getRecent', fx.linked],
      ['sessions:getRecent', fx.plain],
      ['sessions:getRecent', path.join(fx.root, 'missing')],
      ['shell:listExecutables', undefined],
      ['shell:listInstalled', undefined]
    ]

    /** One call on each side, the answers compared. */
    async function same(
      direct: Client,
      through: Client,
      method: string,
      params: unknown,
      normalize: (a: Answer) => Answer = (a) => a
    ): Promise<Answer> {
      const want = normalize(answerOf(await direct.call(method, params)))
      const got = normalize(answerOf(await through.call(method, params)))
      expect(got, `${method} ${JSON.stringify(params)}`).toEqual(want)
      return got
    }

    it('answers every lookup with the server’s frame', async () => {
      const direct = await Client.open(serverPort)
      const through = await Client.open(native!.port)
      // The first answer the server sends shows vornd the socket was admitted.
      await through.call('config:load')
      try {
        for (const [method, params] of readCalls()) await same(direct, through, method, params)
        const installed = await same(direct, through, 'agent:detectInstalled', undefined)
        expect(installed.result).toMatchObject({ claude: true, opencode: true, gemini: true })
        const recent = await same(direct, through, 'sessions:getRecent', fx.project)
        const ids = (recent.result as Array<{ sessionId: string }>).map((s) => s.sessionId)
        // Newest first; a claude and a codex session at one time keep the server's agent order.
        expect(ids).toEqual(['p1', 'g1', 'c1', 'c2', 'o1', 'c3', 'c5', 'x1'])
      } finally {
        direct.close()
        through.close()
      }
      const groups = await counts(native!)
      for (const group of ['agent', 'sessions', 'shell']) {
        expect(groups[group]?.mode).toBe('native')
        expect(groups[group]?.native ?? 0).toBeGreaterThan(0)
      }
    }, 60_000)

    it('lists each agent’s models as the server does, cached and refreshed alike', async () => {
      const direct = await Client.open(serverPort)
      const through = await Client.open(native!.port)
      await through.call('config:load')
      const before = await counts(native!)
      let made = 0
      const models = (params: unknown): Promise<Answer> => {
        made++
        return same(direct, through, 'agent:listModels', params, catalogFetchedAt)
      }
      try {
        for (const agentType of ['claude', 'codex', 'copilot', 'opencode']) {
          const first = await models({ agentType, projectPath: fx.project })
          expect((first.result as { status: string }).status).toBe('ready')
          await models({ agentType, projectPath: fx.project })
        }
        for (const request of [
          { agentType: 'gemini', projectPath: fx.project },
          { agentType: 'shell', projectPath: fx.project },
          { agentType: 'claude', projectPath: fx.project, remoteHostId: 'host-1' },
          { agentType: 'claude', projectPath: '{{context.path}}' },
          { agentType: 'claude' }
        ]) {
          await models(request)
        }
        // A CLI that stops answering: the last list, stale, with the reason.
        const fails = [fx.project, fx.plain].map((dir) => path.join(dir, '.fail-models'))
        for (const fail of fails) fs.writeFileSync(fail, '')
        try {
          const stale = await models({
            agentType: 'claude',
            projectPath: fx.project,
            refresh: true
          })
          expect(stale.result).toMatchObject({ status: 'stale' })
          await models({ agentType: 'claude', projectPath: fx.project })
          await models({ agentType: 'claude', projectPath: fx.project, refresh: true })
          const never = await models({ agentType: 'opencode', projectPath: fx.plain })
          expect(never.result).toMatchObject({ status: 'unavailable' })
        } finally {
          for (const fail of fails) fs.rmSync(fail)
        }
        const back = await models({ agentType: 'claude', projectPath: fx.project, refresh: true })
        expect(back.result).toMatchObject({
          status: 'ready',
          choices: [
            { id: 'opus', label: 'Opus', description: 'Most capable' },
            { id: 'haiku', label: 'Haiku' }
          ]
        })
      } finally {
        direct.close()
        through.close()
      }
      const after = await counts(native!)
      expect((after.agent?.native ?? 0) - (before.agent?.native ?? 0)).toBe(made)
    }, 60_000)

    it('leaves the calls only the server can answer to the server', async () => {
      const through = await Client.open(native!.port)
      await through.call('config:load')
      const before = await counts(native!)
      await through.call('sessions:restored')
      await through.call('sessions:getRecent', 'relative/project')
      await through.call('agent:listModels', { agentType: 'claude', projectPath: 'relative' })
      through.close()
      const after = await counts(native!)
      expect((after.sessions?.forwarded ?? 0) - (before.sessions?.forwarded ?? 0)).toBe(2)
      expect((after.agent?.forwarded ?? 0) - (before.agent?.forwarded ?? 0)).toBe(1)
      expect(after.sessions?.native ?? 0).toBe(before.sessions?.native ?? 0)
    }, 60_000)

    it('shadows every lookup and finds no difference', async () => {
      const direct = await Client.open(serverPort)
      const through = await Client.open(shadow!.port)
      await through.call('config:load')
      try {
        for (const [method, params] of readCalls()) await same(direct, through, method, params)
      } finally {
        direct.close()
        through.close()
      }
      // Shadow answers settle after the server's; wait for the counts to stop moving.
      let groups = await counts(shadow!)
      for (let tries = 0; tries < 50; tries++) {
        await new Promise((r) => setTimeout(r, 100))
        const next = await counts(shadow!)
        const settled = JSON.stringify(next) === JSON.stringify(groups)
        groups = next
        if (settled && tries > 2) break
      }
      for (const group of ['agent', 'sessions', 'shell']) {
        expect(groups[group]?.mode).toBe('shadow')
        expect(groups[group]?.native ?? 0).toBe(0)
        expect(groups[group]?.shadowMismatched ?? 0).toBe(0)
        expect(groups[group]?.shadowMatched ?? 0).toBeGreaterThan(0)
      }
    }, 60_000)
  }
)

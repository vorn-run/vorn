import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'
import type { AiAgentType, TerminalSession } from '@vornrun/shared/types'
import { BOOTSTRAP_ENV_VAR, WS_PORT_FILENAME } from '@vornrun/shared/protocol'
import {
  BytesClient,
  killPid,
  until,
  vorndBinaries,
  vorndSessionsAvailable
} from './helpers/vornd-sessions'

/**
 * The app with the Native daemon switch on, end to end: the real server (run
 * from source, as a process of its own), the real vornd and session holder, and
 * clients connecting through vornd as the desktop does.
 *
 * - RC-T1 through the app: sessions the server creates print while vornd is
 *   killed and started again; each client's stream comes through whole, every
 *   line once and in order.
 * - Closing and reopening the app (vornd ends with it; the server and the
 *   holder do not) keeps every shell and agent running.
 * - A server that restarts finds its terminals in vornd, live under the same
 *   ids, instead of offering them back as restored sessions.
 * - The five agents launch, report their status and resume through vornd. No
 *   agent CLI is installed here, so each is a stand-in script that prints the
 *   prompt the status analysis keys on and the arguments it was started with.
 *
 * Runs in `yarn test:conformance`, which builds vornd and vorn-sessiond; skipped
 * otherwise. Unix only, like the other tests of the session holder here.
 */

const TOKEN = 'app-restart-test-credential'
const AGENTS: AiAgentType[] = ['claude', 'copilot', 'codex', 'opencode', 'gemini']
const repo = path.resolve(__dirname, '..')

/** A stand-in agent: says how it was started, then a prompt, and echoes work until `quit`. */
const STAND_IN = `#!/bin/sh
echo "stand-in $(basename "$0") $*"
while true; do
  printf '> '
  read line || exit 0
  [ "$line" = quit ] && exit 0
  echo "working on $line"
  sleep 0.3
  echo "done with $line"
done
`

let root: string
let dataDir: string

interface Server {
  child: ChildProcess
  port: number
}

/** The server, from source, in its own process: the one the desktop starts. */
async function startServer(): Promise<Server> {
  const portFile = path.join(dataDir, WS_PORT_FILENAME)
  fs.rmSync(portFile, { force: true })
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    // Its own home, so the hooks it installs for agents land nowhere real.
    HOME: root,
    [BOOTSTRAP_ENV_VAR]: TOKEN
  }
  delete env.VITEST
  const log = fs.openSync(path.join(root, 'server.log'), 'a')
  // tsx's loader in this node, so the process is the server itself and a
  // signal reaches it, not a launcher in front of it.
  const child = spawn(
    process.execPath,
    [
      '--import',
      'tsx',
      path.join(repo, 'packages/server/src/cli.ts'),
      'server',
      'serve',
      '--port',
      '0',
      '--data-dir',
      dataDir
    ],
    { cwd: repo, env, stdio: ['ignore', log, log] }
  )
  let port = 0
  await until('the server to listen', () => {
    try {
      // Removed above, so whatever is there now is this server's.
      port = (JSON.parse(fs.readFileSync(portFile, 'utf8')) as { port: number }).port
      return true
    } catch {
      return false
    }
  })
  return { child, port }
}

async function stopProcess(child: ChildProcess, signal: NodeJS.Signals = 'SIGKILL'): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return
  await new Promise<void>((resolve) => {
    child.once('exit', () => resolve())
    child.kill(signal)
  })
}

interface App {
  vornd: ChildProcess
  port: number
}

/** vornd as the app starts it: in front of the server, with its credential, ending with the app. */
async function openApp(server: Server): Promise<App> {
  const vornd = spawn(
    vorndBinaries.vornd!,
    [
      '--upstream',
      `127.0.0.1:${server.port}`,
      '--exit-with-stdin',
      '--sessiond',
      vorndBinaries.sessiond!,
      '--home',
      dataDir
    ],
    {
      stdio: ['pipe', 'pipe', 'inherit'],
      env: { ...process.env, VORND_LOG: process.env.VORND_LOG ?? 'warn', VORND_SERVER_TOKEN: TOKEN }
    }
  )
  const port = await new Promise<number>((resolve, reject) => {
    createInterface({ input: vornd.stdout! }).once('line', (line) =>
      resolve((JSON.parse(line) as { port: number }).port)
    )
    vornd.once('exit', (code) => reject(new Error(`vornd exited with ${code}`)))
  })
  return { vornd, port }
}

/** The app closing: vornd's stdin ends with it. */
async function closeApp(app: App): Promise<void> {
  await new Promise<void>((resolve) => {
    app.vornd.once('exit', () => resolve())
    app.vornd.stdin!.end()
  })
}

async function client(app: App): Promise<BytesClient> {
  const c = new BytesClient()
  await c.connect(app.port, { Authorization: `Bearer ${TOKEN}` })
  return c
}

async function active(c: BytesClient): Promise<TerminalSession[]> {
  return c.call<TerminalSession[]>('terminal:listActive', {})
}

async function statusOf(c: BytesClient, id: string): Promise<string | undefined> {
  return (await active(c)).find((s) => s.id === id)?.status
}

/**
 * Attach `id` through vornd. vornd holds a session once its engine has taken
 * it on from the session holder; an attach before that is the server's to
 * answer, and the server then leaves the stream to vornd (and tells the client
 * to attach again once vornd has it). Waiting for vornd's report to show the
 * session live takes that race out of the test.
 */
async function attachHeld(
  app: App,
  c: BytesClient,
  id: string,
  resume = false
): Promise<{ continued: boolean }> {
  await until(`vornd to hold ${id}`, async () => {
    const res = await fetch(`http://127.0.0.1:${app.port}/vornd/sessions`).catch(() => null)
    if (!res?.ok) return false
    const report = (await res.json()) as { sessions: Array<{ session: string; state: string }> }
    return report.sessions.some((s) => s.session === id && s.state === 'live')
  })
  const answer = await c.attach(id, resume)
  expect(answer.replies).toBe('vornd')
  return answer
}

let sessiondPid: number | null = null

async function holderPid(app: App): Promise<number | null> {
  const res = await fetch(`http://127.0.0.1:${app.port}/vornd/health`)
  const health = (await res.json()) as { sessiond?: { current?: { pid: number } } }
  return health.sessiond?.current?.pid ?? null
}

describe.runIf(vorndSessionsAvailable)('the app with vornd as its process backend', () => {
  let server: Server
  let app: App
  const clients: BytesClient[] = []

  beforeAll(async () => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-app-'))
    dataDir = path.join(root, 'data')
    const bin = path.join(root, 'bin')
    fs.mkdirSync(bin)
    for (const agent of AGENTS) {
      fs.writeFileSync(path.join(bin, agent), STAND_IN, { mode: 0o755 })
    }
    // The switch on, a plain shell, and every agent pointed at its stand-in.
    const db = await import('../packages/server/src/database')
    db.initDatabase(dataDir)
    const config = db.loadConfig()
    config.defaults = {
      ...config.defaults,
      shell: '/bin/sh',
      experimental: { vornd: true }
    }
    config.agentCommands = Object.fromEntries(
      AGENTS.map((a) => [a, { command: path.join(bin, a), args: [] }])
    ) as typeof config.agentCommands
    db.saveConfig(config)
    db.closeDatabase()

    server = await startServer()
    app = await openApp(server)
    sessiondPid = await holderPid(app)
  }, 60_000)

  afterAll(async () => {
    for (const c of clients) c.close()
    if (server) await stopProcess(server.child, 'SIGTERM')
    if (app) await stopProcess(app.vornd)
    killPid(sessiondPid)
    if (process.env.KEEP_VORND_APP_DIR) console.log(`kept ${root}`)
    else fs.rmSync(root, { recursive: true, force: true })
  }, 30_000)

  async function connected(): Promise<BytesClient> {
    const c = await client(app)
    clients.push(c)
    return c
  }

  it('RC-T1 through the app: output survives vornd being killed, each line once', async () => {
    const shells: Array<{ id: string; c: BytesClient }> = []
    for (let k = 0; k < 3; k++) {
      const c = await connected()
      const session = await c.call<TerminalSession>('shell:create', root)
      await attachHeld(app, c, session.id)
      shells.push({ id: session.id, c })
    }
    for (const [k, { id, c }] of shells.entries()) {
      c.notify('terminal:write', {
        id,
        data: `i=0; while [ $i -lt 2000 ]; do echo "s${k} line $i"; i=$((i+1)); [ $((i % 50)) -eq 0 ] && sleep 0.1; done\r`
      })
    }
    // vornd dies twice mid-burst, as a crash would, and the app starts it again.
    for (let kill = 0; kill < 2; kill++) {
      await until('some output', async () =>
        (await shells[0]!.c.text()).includes(`line ${300 + kill * 500}`)
      )
      await stopProcess(app.vornd)
      await new Promise((r) => setTimeout(r, 300))
      app = await openApp(server)
      for (const s of shells) {
        s.c.close()
        await s.c.connect(app.port, { Authorization: `Bearer ${TOKEN}` })
        // From the cursor it had: vornd continues it without a snapshot.
        const resumed = await attachHeld(app, s.c, s.id, true)
        expect(resumed.continued).toBe(true)
      }
    }
    for (const [k, { c }] of shells.entries()) {
      await until(`s${k} to finish`, async () => (await c.text()).includes(`s${k} line 1999`))
      expect(c.broken).toBeNull()
      // Typed before the shell's first prompt, the command comes back ahead of
      // it, and the prompt then sits in front of the first line of output.
      const lines = (await c.text())
        .split('\n')
        .map((l) => l.replace(/^[#$] /, ''))
        .filter((l) => new RegExp(`^s${k} line \\d+$`).test(l))
      expect(lines).toEqual(Array.from({ length: 2000 }, (_, i) => `s${k} line ${i}`))
    }
  }, 120_000)

  it('closing and reopening the app keeps every shell and agent running', async () => {
    const c = await connected()
    const shell = await c.call<TerminalSession>('shell:create', root)
    const agent = await c.call<TerminalSession>('terminal:create', {
      agentType: 'claude',
      projectName: 'proj',
      projectPath: root
    })
    await until(
      'the agent to wait for input',
      async () => (await statusOf(c, agent.id)) === 'waiting'
    )
    const pids = new Map((await active(c)).map((s) => [s.id, s.pid]))

    await closeApp(app)
    // Nothing is linked now; the holder keeps both running.
    expect(process.kill(pids.get(shell.id)!, 0)).toBe(true)
    expect(process.kill(pids.get(agent.id)!, 0)).toBe(true)
    app = await openApp(server)

    const after = await connected()
    const listed = await active(after)
    expect(listed.find((s) => s.id === shell.id)?.pid).toBe(pids.get(shell.id))
    expect(listed.find((s) => s.id === agent.id)?.pid).toBe(pids.get(agent.id))
    await attachHeld(app, after, agent.id)
    after.notify('terminal:write', { id: agent.id, data: 'the tests\r' })
    await until('the agent to work', async () =>
      (await after.text()).includes('done with the tests')
    )
    await until('it to wait again', async () => (await statusOf(after, agent.id)) === 'waiting')
  }, 60_000)

  it('a restarted server takes its terminals back from vornd, under the same ids', async () => {
    const c = await connected()
    const shell = await c.call<TerminalSession>('shell:create', root)
    await attachHeld(app, c, shell.id)
    c.notify('terminal:write', { id: shell.id, data: 'echo before-restart\r' })
    await until('the echo', async () => (await c.text()).includes('before-restart'))
    // The record is saved on a short debounce.
    await new Promise((r) => setTimeout(r, 1500))

    // The server dies; the app goes with it and comes back with a new server.
    await stopProcess(server.child)
    await closeApp(app)
    server = await startServer()
    app = await openApp(server)

    const after = await connected()
    await until('the terminal to be live again', async () =>
      (await active(after)).some((s) => s.id === shell.id && s.status === 'running')
    )
    const restored = await after.call<Array<{ session: { id: string } }>>('sessions:restored', {})
    expect(restored.map((r) => r.session.id)).not.toContain(shell.id)
    await attachHeld(app, after, shell.id)
    after.notify('terminal:write', { id: shell.id, data: 'echo after-restart\r' })
    await until('the echo', async () => (await after.text()).includes('after-restart'))
  }, 90_000)

  it('all five agents launch, report status and resume through vornd', async () => {
    const c = await connected()
    for (const agentType of AGENTS) {
      const agent = await c.call<TerminalSession>('terminal:create', {
        agentType,
        projectName: 'proj',
        projectPath: root
      })
      const watcher = await connected()
      await attachHeld(app, watcher, agent.id)
      await until(`${agentType} to start`, async () =>
        (await watcher.text()).includes(`stand-in ${agentType}`)
      )
      await until(`${agentType} to wait`, async () => (await statusOf(c, agent.id)) === 'waiting')
      watcher.notify('terminal:write', { id: agent.id, data: 'step one\r' })
      await until(`${agentType} to work`, async () =>
        (await watcher.text()).includes('done with step one')
      )
      await until(
        `${agentType} to wait again`,
        async () => (await statusOf(c, agent.id)) === 'waiting'
      )

      // The agent and its shell end; the session is resumed under its id.
      watcher.notify('terminal:write', { id: agent.id, data: 'quit\rexit\r' })
      await until(`${agentType} to end`, async () =>
        (await active(c)).some((s) => s.id === agent.id && s.status === 'idle')
      )
      const resumed = await c.call<{ ok: boolean; session?: TerminalSession }>('sessions:resume', {
        id: agent.id
      })
      expect(resumed.ok).toBe(true)
      expect(resumed.session?.id).toBe(agent.id)
      const again = await connected()
      await until(`${agentType} to be held again`, async () => {
        try {
          await again.attach(agent.id)
          return (await again.text()).includes(`stand-in ${agentType}`)
        } catch {
          return false
        }
      })
      // A prompt typed ahead of may sit in front of it.
      const started = (await again.text())
        .split('\n')
        .find((l) => l.includes(`stand-in ${agentType}`))!
      if (agentType === 'claude' || agentType === 'copilot') {
        // A pinned conversation resumes exactly.
        expect(started).toContain(`--resume ${agent.agentSessionId}`)
      }
      await until(
        `${agentType} to wait after resuming`,
        async () => (await statusOf(c, agent.id)) === 'waiting'
      )
    }
  }, 180_000)
})

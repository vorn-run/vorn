import { describe, it, expect, vi, beforeAll, beforeEach, afterEach, afterAll } from 'vitest'
import { EventEmitter } from 'node:events'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { CreateTerminalPayload, TerminalSession } from '@vornrun/shared/types'

/**
 * The server with vornd as its process backend (the Native daemon switch on):
 * terminals and piped agents start in vornd's session holder, their output
 * comes back as records, and what vornd's analysis reports comes back as
 * effects, each acted on by the rule the Session Recovery Contract gives it
 * (§7): states applied when newer, notifications and workflow triggers once
 * per effect id, bells never twice.
 *
 * vornd is `LoopbackVornd`, which answers the link in this process over the
 * fake node-pty and child_process below; the real vornd is in
 * `vornd-app-restart.test.ts` and the Rust tests of the link.
 */

// The classes are returned for their types only (`FakePtyInstance`, `FakeChildInstance`).
// eslint-disable-next-line @typescript-eslint/no-unused-vars
const { spawnMock, childSpawnMock, FakePty, FakeChild } = vi.hoisted(() => {
  type DataHandler = (data: string) => void
  type ExitHandler = (e: { exitCode: number; signal?: number }) => void
  let nextPid = 7000

  class FakePty {
    pid = nextPid++
    written: string[] = []
    killedWith: string[] = []
    resizes: Array<[number, number]> = []
    env: Record<string, string> = {}
    private dataHandlers: DataHandler[] = []
    private exitHandlers: ExitHandler[] = []
    write(data: string): void {
      this.written.push(data)
    }
    resize(cols: number, rows: number): void {
      this.resizes.push([cols, rows])
    }
    kill(signal?: string): void {
      this.killedWith.push(signal ?? 'SIGHUP')
    }
    onData(cb: DataHandler): { dispose: () => void } {
      this.dataHandlers.push(cb)
      return { dispose: () => (this.dataHandlers = this.dataHandlers.filter((h) => h !== cb)) }
    }
    onExit(cb: ExitHandler): { dispose: () => void } {
      this.exitHandlers.push(cb)
      return { dispose: () => (this.exitHandlers = this.exitHandlers.filter((h) => h !== cb)) }
    }
    emitData(data: string): void {
      for (const h of [...this.dataHandlers]) h(data)
    }
    emitExit(exitCode = 0, signal?: number): void {
      for (const h of [...this.exitHandlers]) h({ exitCode, signal })
    }
  }

  class FakeChild {
    pid = nextPid++
    stdinWritten: string[] = []
    stdinEnded = false
    killedWith: string[] = []
    private handlers = new Map<string, Array<(...a: unknown[]) => void>>()
    stdout = {
      handlers: [] as Array<(b: Buffer) => void>,
      on(_: string, cb: (b: Buffer) => void) {
        this.handlers.push(cb)
      }
    }
    stderr = { on: (): void => {} }
    stdin = {
      on: (): void => {},
      write: (d: string): void => {
        this.stdinWritten.push(d)
      },
      end: (): void => {
        this.stdinEnded = true
      }
    }
    on(event: string, cb: (...a: unknown[]) => void): this {
      const list = this.handlers.get(event) ?? []
      list.push(cb)
      this.handlers.set(event, list)
      return this
    }
    kill(signal?: string): boolean {
      this.killedWith.push(signal ?? 'SIGTERM')
      return true
    }
    out(data: string): void {
      for (const h of this.stdout.handlers) h(Buffer.from(data))
    }
    exit(code: number | null): void {
      for (const h of this.handlers.get('exit') ?? []) h(code)
    }
  }

  return {
    spawnMock: vi.fn(() => new FakePty()),
    childSpawnMock: vi.fn(() => new FakeChild()),
    FakePty,
    FakeChild
  }
})

type FakePtyInstance = InstanceType<typeof FakePty>
type FakeChildInstance = InstanceType<typeof FakeChild>

vi.mock('../packages/server/node_modules/node-pty', () => ({
  default: { spawn: spawnMock },
  spawn: spawnMock
}))
vi.mock('node:child_process', async (importOriginal) => ({
  ...(await importOriginal<typeof import('node:child_process')>()),
  spawn: childSpawnMock
}))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: vi.fn(() => ({ defaults: { shell: '/bin/zsh', minimalShellPrompt: true } }))
  }
}))
vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(async () => 'main'),
  getGitHead: vi.fn(async () => 'cafe0000'),
  checkoutBranch: vi.fn(async () => {}),
  createWorktree: vi.fn(),
  extractWorktreeName: vi.fn((p: string) => path.basename(p)),
  isGitRepo: vi.fn(async () => false)
}))
vi.mock('../packages/server/src/shell-integration', () => ({
  getShellIntegration: vi.fn(() => ({ env: {} }))
}))
vi.mock('../packages/server/src/agent-launch', () => ({
  buildAgentLaunchLine: vi.fn((payload: CreateTerminalPayload) => `${payload.agentType}-launch`),
  buildHeadlessSpawnArgs: vi.fn(() => ({ command: 'claude', args: ['-p'], stdin: 'fix it' }))
}))
vi.mock('../packages/server/src/process-utils', async () => ({
  ...(await vi.importActual<typeof import('../packages/server/src/process-utils')>(
    '../packages/server/src/process-utils'
  )),
  getSafeEnv: () => ({ HOME: '/home/user', PATH: '/usr/bin' }),
  getLaunchEnv: () => ({ HOME: '/home/user', PATH: '/usr/bin' })
}))

import { IPC } from '@vornrun/shared/types'
import { ptyManager } from '../packages/server/src/pty-manager'
import { headlessManager } from '../packages/server/src/headless-manager'
import { clientRegistry } from '../packages/server/src/broadcast'
import { closeDatabase, initDatabase } from '../packages/server/src/database'
import { resetEffectMemory } from '../packages/server/src/effect-receipts'
import { setNativeDaemonOverride } from '../packages/server/src/process-backend'
import { listRestored, resetRestored, seedRestored } from '../packages/server/src/restored-sessions'
import { wireVorndBackend } from '../packages/server/src/vornd-backend'
import { vorndLink } from '../packages/server/src/vornd-link'
import { VorndProcess } from '../packages/server/src/vornd-process'
import { installLoopbackVornd, type LoopbackVornd } from './helpers/loopback-vornd'

let vornd: LoopbackVornd
let dataDir: string
const messages: Array<{ channel: string; payload: unknown }> = []
const headless: Array<{ channel: string; payload: unknown }> = []
const adopted: TerminalSession[] = []

const onPty = (channel: string, payload: unknown): void => {
  messages.push({ channel, payload })
}
const onHeadless = (channel: string, payload: unknown): void => {
  headless.push({ channel, payload })
}

function on(channel: string): unknown[] {
  return messages.filter((m) => m.channel === channel).map((m) => m.payload)
}

function dataOf(id: string): string {
  return (on(IPC.TERMINAL_DATA) as Array<{ id: string; data: string }>)
    .filter((m) => m.id === id)
    .map((m) => m.data)
    .join('')
}

function lastPty(): FakePtyInstance {
  const r = spawnMock.mock.results
  return r[r.length - 1]!.value as FakePtyInstance
}

function lastChild(): FakeChildInstance {
  const r = childSpawnMock.mock.results
  return r[r.length - 1]!.value as FakeChildInstance
}

/** Past the 8 ms output hold. */
const afterFlush = (): Promise<void> => new Promise((r) => setTimeout(r, 40))

async function until(what: string, check: () => boolean): Promise<void> {
  const start = Date.now()
  while (!check()) {
    if (Date.now() - start > 3000) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 5))
  }
}

beforeAll(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-backend-'))
  initDatabase(dataDir)
  wireVorndBackend({ adopted: (s) => adopted.push(s) })
})

afterAll(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

beforeEach(() => {
  messages.length = 0
  headless.length = 0
  adopted.length = 0
  spawnMock.mockClear()
  childSpawnMock.mockClear()
  resetRestored()
  ptyManager.on('client-message', onPty)
  headlessManager.on('client-message', onHeadless)
  vornd = installLoopbackVornd()
})

afterEach(() => {
  for (const s of ptyManager.getActiveSessions()) ptyManager.killPty(s.id)
  ptyManager.off('client-message', onPty)
  headlessManager.off('client-message', onHeadless)
  vornd.uninstall()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('terminals start in the session holder', () => {
  it('under the session id, with its own VORN_SESSION_ID, nothing spawned by the server', async () => {
    const session = ptyManager.createShellPty('/tmp')
    const spawn = vornd.calls.find((c) => c.method === 'vornd:spawn')!
    expect(spawn.params.id).toBe(session.id)
    expect(spawn.params.argv).toEqual(['/bin/zsh', '-l'])
    expect((spawn.params.env as Record<string, string>).VORN_SESSION_ID).toBe(session.id)
    expect(spawn.params.cwd).toBe('/tmp')
    // node-pty ran once, in the holder's place, not for the server.
    expect(spawnMock).toHaveBeenCalledTimes(1)
    // The pid is the holder's answer.
    await new Promise((r) => setImmediate(r))
    expect(ptyManager.getActiveSessions().find((s) => s.id === session.id)?.pid).toBe(lastPty().pid)
    expect(ptyManager.isBackendHeld(session.id)).toBe(true)
  })

  it('agents too, typing their launch line once the holder has them', async () => {
    vi.useFakeTimers()
    const session = await ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    })
    vi.advanceTimersByTime(300)
    expect(lastPty().written).toEqual(['claude-launch\r'])
    const write = vornd.calls.find((c) => c.method === 'vornd:write')!
    expect(write.params).toEqual({ id: session.id, data: 'claude-launch\r' })
  })

  it('holds input until the holder has the session, then sends it once', async () => {
    vornd.spawnDelayMs = 30
    const session = ptyManager.createShellPty('/tmp')
    ptyManager.writeToPty(session.id, 'ls\r')
    expect(spawnMock).not.toHaveBeenCalled()
    await until('the spawn', () => spawnMock.mock.calls.length === 1)
    await until('the input', () => lastPty().written.length > 0)
    expect(lastPty().written).toEqual(['ls\r'])
  })

  it('is the server’s own again with the switch off, linked or not', () => {
    setNativeDaemonOverride(false)
    const session = ptyManager.createShellPty('/tmp')
    expect(vornd.calls.some((c) => c.method === 'vornd:spawn')).toBe(false)
    expect(ptyManager.isBackendHeld(session.id)).toBe(false)
  })

  it('is the server’s own with the switch on and no vornd linked', () => {
    vornd.unlink()
    const session = ptyManager.createShellPty('/tmp')
    expect(vornd.calls.some((c) => c.method === 'vornd:spawn')).toBe(false)
    expect(ptyManager.isBackendHeld(session.id)).toBe(false)
  })

  it('a start that fails is the session saying why and exiting', async () => {
    spawnMock.mockImplementationOnce(() => {
      throw new Error('posix_spawnp failed')
    })
    const session = ptyManager.createShellPty('/tmp')
    await afterFlush()
    expect(dataOf(session.id)).toContain('posix_spawnp failed')
    expect(on(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 1 }])
  })
})

describe('output, resizes and the exit', () => {
  it('reaches clients once, a replay after a vornd restart included', async () => {
    const session = ptyManager.createShellPty('/tmp')
    const fake = lastPty()
    fake.emitData('one\r\n')
    await afterFlush()
    expect(dataOf(session.id)).toBe('one\r\n')

    // vornd dies and comes back; it replays from its checkpoint at the start.
    vornd.unlink()
    vornd.link()
    vornd.replay(session.id, 0)
    fake.emitData('two\r\n')
    await afterFlush()
    expect(dataOf(session.id)).toBe('one\r\ntwo\r\n')
  })

  it('keeps a character split across two records whole', async () => {
    const session = ptyManager.createShellPty('/tmp')
    const held = vornd.sessions.get(session.id)!
    const bytes = Buffer.from('é')
    // Straight into the records, split inside the character.
    const send = (b: Buffer, rseq: number, offset: number): void =>
      vorndLink.receive({
        method: 'vornd:records',
        params: {
          id: session.id,
          records: [{ epoch: held.epoch, rseq, offset, data: b.toString('base64') }]
        }
      })
    send(bytes.subarray(0, 1), 0, 0)
    send(bytes.subarray(1), 1, 1)
    await afterFlush()
    expect(dataOf(session.id)).toBe('é')
  })

  it('moves the session to a size recorded in its log, once', async () => {
    const session = ptyManager.createShellPty('/tmp')
    ptyManager.resizePty(session.id, 120, 40)
    // Held until the holder has the session, then sent.
    await new Promise((r) => setImmediate(r))
    expect(lastPty().resizes).toEqual([[120, 40]])
    expect(ptyManager.getActiveSessions()[0]).toMatchObject({ cols: 120, rows: 40 })
    // The same size again goes nowhere.
    ptyManager.resizePty(session.id, 120, 40)
    expect(vornd.calls.filter((c) => c.method === 'vornd:resize')).toHaveLength(1)
  })

  it('kills by a signal through the holder, and hears the exit back', async () => {
    const session = ptyManager.createShellPty('/tmp')
    const fake = lastPty()
    ptyManager.killPty(session.id)
    await new Promise((r) => setImmediate(r))
    const signal = vornd.calls.find(
      (c) => c.method === 'vornd:signal' && c.params.id === session.id
    )
    expect(signal?.params).toEqual({
      id: session.id,
      signal: 'hup'
    })
    expect(fake.killedWith).toEqual(['SIGHUP'])
  })

  it('ends a session whose program exits, with its code', async () => {
    const session = ptyManager.createShellPty('/tmp')
    lastPty().emitData('bye\r\n')
    lastPty().emitExit(3)
    await afterFlush()
    expect(dataOf(session.id)).toBe('bye\r\n')
    expect(on(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 3 }])
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
    expect(ptyManager.isBackendHeld(session.id)).toBe(false)
  })

  it('takes an exit effect alone when its records never come', async () => {
    vi.useFakeTimers()
    const session = ptyManager.createShellPty('/tmp')
    const held = vornd.sessions.get(session.id)!
    vorndLink.receive({
      method: 'vornd:effect',
      params: vornd.effectFor(held, 9, 0, { kind: 'exit', code: 5, signal: null })
    })
    expect(on(IPC.TERMINAL_EXIT)).toEqual([])
    vi.advanceTimersByTime(2_000)
    expect(on(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 5 }])
  })
})

describe('agent status comes from vornd', () => {
  async function agent(): Promise<TerminalSession> {
    return ptyManager.createPty({ agentType: 'claude', projectName: 'p', projectPath: '/tmp' })
  }

  function status(session: TerminalSession, code: number, rseq: number, index = 0): void {
    const held = vornd.sessions.get(session.id)!
    vorndLink.receive({
      method: 'vornd:effect',
      params: vornd.effectFor(held, rseq, index, { kind: 'status', status: code })
    })
  }

  const statusOf = (id: string): string | undefined =>
    ptyManager.getActiveSessions().find((s) => s.id === id)?.status

  it('applied only when newer than the last one applied', async () => {
    const session = await agent()
    status(session, 2, 5)
    expect(statusOf(session.id)).toBe('waiting')
    // From a replay, or late: an older place changes nothing.
    status(session, 1, 4)
    status(session, 1, 5, 0)
    expect(statusOf(session.id)).toBe('waiting')
    status(session, 1, 5, 1)
    expect(statusOf(session.id)).toBe('running')
  })

  it('not for a session whose status comes from hooks', async () => {
    const session = await agent()
    ptyManager.promoteToHookStatus(session.id)
    ptyManager.updateSessionStatus(session.id, 'running')
    status(session, 2, 1)
    expect(statusOf(session.id)).toBe('running')
  })

  it('converges after a vornd restart replays it', async () => {
    const session = await agent()
    status(session, 1, 1)
    status(session, 2, 2)
    vornd.unlink()
    vornd.link()
    status(session, 1, 1)
    status(session, 2, 2)
    expect(statusOf(session.id)).toBe('waiting')
  })
})

/**
 * RC-T22 and TP-T27 as the server sees them: vornd killed (a) after a record
 * holding a bell was sent, (b) after a notification was emitted and before the
 * next checkpoint, (c) after a workflow trigger was sent and before its receipt,
 * (d) after a clipboard write. vornd's half (what it sends again after a
 * restart) is the Rust link test; this is the receiving half.
 */
describe('effects across a vornd crash', () => {
  /** What was broadcast as a notification, read each time it is called. */
  function notifications(): () => unknown[] {
    const spy = vi.spyOn(clientRegistry, 'broadcast')
    return () => spy.mock.calls.filter(([m]) => m === IPC.TERMINAL_NOTIFY).map(([, p]) => p)
  }

  it('(a) rings the bell at most once', async () => {
    const session = ptyManager.createShellPty('/tmp')
    lastPty().emitData('ding\x07')
    await afterFlush()
    vornd.unlink()
    vornd.link()
    vornd.replay(session.id, 0)
    await afterFlush()
    expect(on(IPC.TERMINAL_BELL)).toEqual([{ id: session.id }])
  })

  it('(b) shows a notification once though it is emitted twice (TP-T27)', async () => {
    const shown = notifications()
    const session = ptyManager.createShellPty('/tmp')
    const held = vornd.sessions.get(session.id)!
    const notify = vornd.effectFor(held, 3, 0, { kind: 'notify', title: '', body: 'build done' })
    vornd.effect(held, notify)
    expect(shown()).toEqual([{ id: session.id, title: '', body: 'build done' }])
    vornd.unlink()
    vornd.link()
    vornd.replay(session.id, 0)
    expect(shown()).toHaveLength(1)

    // And after this server restarts: the receipt is in the database.
    resetEffectMemory()
    vornd.effect(held, notify)
    expect(shown()).toHaveLength(1)
    // A different notification is a different one.
    vornd.effect(held, vornd.effectFor(held, 4, 0, { kind: 'notify', title: '', body: 'again' }))
    expect(shown()).toHaveLength(2)
  })

  it('(c) runs a workflow step once though its trigger is delivered twice', async () => {
    const agent = await headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp',
      initialPrompt: 'fix it',
      headless: true
    })
    const spawn = vornd.calls.find((c) => c.method === 'vornd:spawn')!
    expect(spawn.params).toMatchObject({ id: agent.id, piped: true, stdin: 'fix it' })
    // The prompt reached the program through the holder, and stdin was closed.
    expect(lastChild().stdinWritten).toEqual(['fix it'])
    expect(lastChild().stdinEnded).toBe(true)

    lastChild().out('{"result":"done"}\n')
    lastChild().exit(0)
    const exits = (): unknown[] =>
      headless.filter((m) => m.channel === IPC.HEADLESS_EXIT).map((m) => m.payload)
    expect(exits()).toEqual([{ id: agent.id, exitCode: 0 }])
    expect(headlessManager.getOutput(agent.id).join('\n')).toContain('"result":"done"')

    // vornd restarts and replays the exit: delivered twice, run once.
    vornd.unlink()
    vornd.link()
    vornd.replay(agent.id, 0)
    expect(exits()).toHaveLength(1)

    // This server restarts too and finds the session again before vornd let
    // it go: the receipt stops the exit from starting anything.
    resetEffectMemory()
    const held = vornd.sessions.get(agent.id)!
    const again = VorndProcess.adopt({ id: agent.id, kind: 'piped', pid: 1, cursor: null }, null)
    const heard: Array<{ first?: boolean }> = []
    again.onExit((e) => heard.push(e))
    vornd.replay(agent.id, 0)
    expect(heard).toHaveLength(1)
    expect(heard[0]!.first).toBe(false)
    expect(held.exited).toEqual({ code: 0, signal: null })
  })

  it('(d) never handles a clipboard write, so it is never repeated here', async () => {
    const spy = vi.spyOn(clientRegistry, 'broadcast')
    const session = ptyManager.createShellPty('/tmp')
    const held = vornd.sessions.get(session.id)!
    vornd.effect(held, vornd.effectFor(held, 1, 0, { kind: 'clipboard' }))
    vornd.replay(session.id, 0)
    expect(spy.mock.calls.filter(([m]) => m !== IPC.TERMINAL_DATA)).toEqual([])
  })

  it('the exit converges whatever was delivered twice', async () => {
    const session = ptyManager.createShellPty('/tmp')
    lastPty().emitExit(7)
    vornd.unlink()
    vornd.link()
    vornd.replay(session.id, 0)
    await afterFlush()
    expect(on(IPC.TERMINAL_EXIT)).toEqual([{ id: session.id, exitCode: 7 }])
  })
})

describe('after this server restarts, its terminals come back from vornd', () => {
  function saved(id: string): TerminalSession {
    return {
      id,
      agentType: 'claude',
      projectName: 'proj',
      projectPath: '/tmp/proj',
      status: 'idle',
      createdAt: 1,
      pid: 1,
      displayName: 'Fix the tests',
      groupId: 'g1',
      savedAt: Date.now()
    }
  }

  it('live, under their own ids, instead of offered as restored', async () => {
    // A session the previous run started, still running in the holder.
    const id = 'f7c1d1a2-0000-4000-8000-000000000001'
    await vorndLink.spawn({ id, argv: ['/bin/zsh'], cwd: '/tmp', env: {}, cols: 100, rows: 30 })
    const fake = lastPty()
    vornd.unlink()
    vornd.waitForFollow = true

    seedRestored([saved(id)])
    expect(listRestored().map((r) => r.session.id)).toEqual([id])
    vornd.link()
    await until('the session to be taken on', () => adopted.length === 1)
    expect(adopted[0]).toMatchObject({
      id,
      displayName: 'Fix the tests',
      groupId: 'g1',
      status: 'running',
      cols: 100,
      rows: 30,
      pid: fake.pid
    })
    expect(listRestored()).toEqual([])
    expect(ptyManager.hasLivePty(id)).toBe(true)
    await until('follow', () => vornd.calls.some((c) => c.method === 'vornd:follow'))

    fake.emitData('still here\r\n')
    await afterFlush()
    expect(dataOf(id)).toBe('still here\r\n')
    ptyManager.writeToPty(id, 'x')
    expect(fake.written).toEqual(['x'])
  })

  it('a terminal vornd no longer holds has ended', async () => {
    const session = ptyManager.createShellPty('/tmp')
    vornd.unlink()
    vornd.drop(session.id)
    vornd.waitForFollow = true
    vornd.link()
    await until('the exit', () => on(IPC.TERMINAL_EXIT).length === 1)
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
  })
})

describe('clients connected through vornd', () => {
  class Client extends EventEmitter {
    OPEN = 1
    readyState = 1
    bufferedAmount = 0
    sent: string[] = []
    send(m: string): void {
      this.sent.push(m)
    }
  }

  it('hear the terminals vornd holds from vornd alone, everything else from here', () => {
    const viaVornd = new Client()
    const direct = new Client()
    clientRegistry.add(viaVornd as never, undefined, true)
    clientRegistry.add(direct as never)
    const session = ptyManager.createShellPty('/tmp')
    setNativeDaemonOverride(false)
    const own = ptyManager.createShellPty('/tmp')
    try {
      for (const id of [session.id, own.id]) {
        clientRegistry.broadcast(IPC.TERMINAL_DATA, { id, data: 'x', seq: 1 }, id)
        clientRegistry.broadcast(IPC.TERMINAL_BELL, { id }, id)
        clientRegistry.broadcast(IPC.TERMINAL_EXIT, { id, exitCode: 0 }, id)
      }
      clientRegistry.broadcast(IPC.SESSION_UPDATED, session, session.id)
      const methods = (c: Client): string[] =>
        c.sent.map((m) => `${JSON.parse(m).method}#${JSON.parse(m).params.id}`)
      expect(methods(direct)).toHaveLength(7)
      expect(methods(viaVornd)).toEqual([
        `${IPC.TERMINAL_DATA}#${own.id}`,
        `${IPC.TERMINAL_BELL}#${own.id}`,
        `${IPC.TERMINAL_EXIT}#${own.id}`,
        `${IPC.SESSION_UPDATED}#${session.id}`
      ])
    } finally {
      clientRegistry.remove(viaVornd as never)
      clientRegistry.remove(direct as never)
    }
  })
})

describe('the review fixes', () => {
  it('sends a terminal\u2019s last bytes and its exit to clients through vornd once, from vornd', async () => {
    const viaVornd = new (class extends EventEmitter {
      OPEN = 1
      readyState = 1
      bufferedAmount = 0
      sent: string[] = []
      send(m: string): void {
        this.sent.push(m)
      }
    })()
    clientRegistry.add(viaVornd as never, undefined, true)
    const forward = (channel: string, payload: unknown): void =>
      clientRegistry.broadcast(channel, payload, (payload as { id?: string }).id)
    ptyManager.on('client-message', forward)
    try {
      const session = ptyManager.createShellPty('/tmp')
      // The first small read goes at once; the second is still held when the
      // program exits, and is drained by the exit.
      lastPty().emitData('a')
      lastPty().emitData('bye\r\n')
      lastPty().emitExit(0)
      await afterFlush()
      expect(dataOf(session.id)).toBe('abye\r\n')
      const mine = viaVornd.sent.filter((m) => m.includes(session.id))
      expect(mine).toEqual([])
    } finally {
      ptyManager.off('client-message', forward)
      clientRegistry.remove(viaVornd as never)
    }
  })

  it('keeps what the server typed while no vornd was linked, and sends it in order once one is', async () => {
    vi.useFakeTimers()
    const session = await ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    })
    const fake = lastPty()
    // The app closes before the launch line is typed.
    vornd.unlink()
    vi.advanceTimersByTime(300)
    ptyManager.writeToPty(session.id, 'next\r')
    expect(fake.written).toEqual([])
    vi.useRealTimers()
    vornd.link()
    await until('the held input', () => fake.written.length === 2)
    expect(fake.written).toEqual(['claude-launch\r', 'next\r'])
  })

  it('sends a signal asked for while no vornd was linked once one is', async () => {
    const session = ptyManager.createShellPty('/tmp')
    await ptyManager.whenStarted(session.id)
    const fake = lastPty()
    vornd.unlink()
    ptyManager.killPty(session.id)
    await new Promise((r) => setImmediate(r))
    expect(fake.killedWith).toEqual([])
    vornd.link()
    await until('the signal', () => fake.killedWith.length === 1)
    expect(fake.killedWith).toEqual(['SIGHUP'])
  })

  it('tells clients through vornd to attach again once vornd holds the sessions', async () => {
    const client = (): EventEmitter & { sent: string[] } =>
      Object.assign(new EventEmitter(), {
        OPEN: 1,
        readyState: 1,
        bufferedAmount: 0,
        sent: [] as string[],
        send(this: { sent: string[] }, m: string) {
          this.sent.push(m)
        }
      })
    const viaVornd = client()
    const direct = client()
    clientRegistry.add(viaVornd as never, undefined, true)
    clientRegistry.add(direct as never)
    try {
      const session = ptyManager.createShellPty('/tmp')
      await new Promise((r) => setImmediate(r))
      vornd.unlink()
      vornd.link()
      await until('the resync', () => viaVornd.sent.length > 0)
      const mine = viaVornd.sent.map((m) => JSON.parse(m)).filter((m) => m.params.id === session.id)
      expect(mine).toEqual([
        { jsonrpc: '2.0', method: 'terminal:resync', params: { id: session.id, reason: 'vornd' } }
      ])
      expect(direct.sent).toEqual([])
    } finally {
      clientRegistry.remove(viaVornd as never)
      clientRegistry.remove(direct as never)
    }
  })

  it('ends a terminal whose session holder died once vornd connects to another', async () => {
    const session = ptyManager.createShellPty('/tmp')
    await new Promise((r) => setImmediate(r))
    vornd.drop(session.id)
    vornd.heldChanged()
    await until('the exit', () => on(IPC.TERMINAL_EXIT).length === 1)
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
  })

  it('sends a size asked for while nothing was linked once a vornd links', async () => {
    const session = ptyManager.createShellPty('/tmp')
    await new Promise((r) => setImmediate(r))
    const fake = lastPty()
    vornd.unlink()
    ptyManager.resizePty(session.id, 120, 40)
    expect(fake.resizes).toEqual([])
    vornd.link()
    await until('the resize', () => fake.resizes.length === 1)
    expect(fake.resizes).toEqual([[120, 40]])
    expect(ptyManager.getActiveSessions().find((s) => s.id === session.id)).toMatchObject({
      cols: 120,
      rows: 40
    })
  })

  it('keeps a session whose spawn answer was lost with the link, and takes it at the next link', async () => {
    vornd.dropLinkOnSpawn = true
    const session = ptyManager.createShellPty('/tmp')
    await ptyManager.whenStarted(session.id)
    expect(on(IPC.TERMINAL_EXIT)).toEqual([])
    const fake = lastPty()
    ptyManager.writeToPty(session.id, 'ls\r')
    vornd.link()
    await until('the input', () => fake.written.length > 0)
    expect(fake.written).toEqual(['ls\r'])
    // The same session, not a second one recovered beside it.
    expect(spawnMock).toHaveBeenCalledTimes(1)
    expect(adopted).toEqual([])
    expect(ptyManager.getActiveSessions().map((s) => s.id)).toEqual([session.id])
  })

  it('spawns again when the lost spawn never reached the holder', async () => {
    vornd.dropLinkOnSpawn = true
    const session = ptyManager.createShellPty('/tmp')
    await ptyManager.whenStarted(session.id)
    vornd.drop(session.id)
    vornd.link()
    await until('a second spawn', () => spawnMock.mock.calls.length === 2)
    await until(
      'it to start',
      () => ptyManager.hasLivePty(session.id) && vornd.sessions.has(session.id)
    )
    expect(on(IPC.TERMINAL_EXIT)).toEqual([])
  })
})

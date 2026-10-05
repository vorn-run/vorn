import { describe, it, expect, vi, beforeAll, afterAll, beforeEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { CreateTerminalPayload } from '@vornrun/shared/types'

/**
 * The terminals and headless agents of the app, against a stand-in vornd: what each asks vornd to do, and what each does
 * with what vornd tells it. The same paths run against the real vornd in
 * `vornd-app-sessions.test.ts` when the binaries are built.
 */

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: vi.fn(() => ({ defaults: { shell: '/bin/sh' } }))
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
vi.mock('../packages/server/src/agent-launch', async () => {
  const actual = await vi.importActual<typeof import('../packages/server/src/agent-launch')>(
    '../packages/server/src/agent-launch'
  )
  return {
    ...actual,
    buildAgentLaunchLine: vi.fn((payload: CreateTerminalPayload) => `${payload.agentType}-launch`)
  }
})
vi.mock('../packages/server/src/resolve-executable', () => ({
  findOnPath: (name: string) => (name === 'claude' ? '/opt/agents/bin/claude' : null)
}))
vi.mock('../packages/server/src/process-utils', async () => {
  const actual = await vi.importActual<typeof import('../packages/server/src/process-utils')>(
    '../packages/server/src/process-utils'
  )
  return {
    ...actual,
    getSafeEnv: () => ({ HOME: '/home/user', PATH: '/usr/bin' }),
    getLaunchEnv: () => ({ HOME: '/home/user', PATH: '/usr/bin' })
  }
})

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { IPC } from '../packages/shared/src/types'
import { ptyManager } from '../packages/server/src/pty-manager'
import { headlessManager } from '../packages/server/src/headless-manager'
import { vorndSessions } from '../packages/server/src/vornd-sessions'
import { FakeVornd, effect } from './helpers/fake-vornd'
import { until } from './helpers/vornd-sessions'

let dataDir: string
let fake: FakeVornd
const messages: Array<{ channel: string; payload: Record<string, unknown> }> = []
const record = (channel: string, payload: Record<string, unknown>): void => {
  messages.push({ channel, payload })
}
const told = (channel: string, id: string): Array<Record<string, unknown>> =>
  messages.filter((m) => m.channel === channel && m.payload.id === id).map((m) => m.payload)

beforeAll(async () => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vornd-backends-'))
  initDatabase(dataDir)
  fake = new FakeVornd(dataDir)
  await fake.start()
  expect(await vorndSessions.connect(fake.endpoint)).toBe(true)
  ptyManager.on('client-message', record)
  headlessManager.on('client-message', record)
})

afterAll(async () => {
  ptyManager.off('client-message', record)
  headlessManager.off('client-message', record)
  vorndSessions.close()
  await fake.stop()
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

beforeEach(() => {
  messages.length = 0
})

/** The spawn vornd was asked for under `id`. */
function spawned(id: string): Record<string, unknown> | undefined {
  return fake.made('vornd:spawn').find((p) => p.name === id)
}

describe('terminals in vornd', () => {
  it('start a shell there, under its own id, with its pid once vornd answers', async () => {
    const session = ptyManager.createShellPty('/work')
    await until('the pid', () => session.pid > 0)
    const spec = spawned(session.id)!
    expect(spec.cwd).toBe('/work')
    expect((spec.argv as string[])[0]).toBe('/bin/sh')
    expect((spec.env as Record<string, string>).VORN_SESSION_ID).toBe(session.id)

    // Its cwd as vornd parsed it.
    fake.send('vornd:effect', effect(session.id, 'cwd', 3, { cwd: '/work/sub' }))
    await until('the cwd', () => session.shellCwd === '/work/sub')

    // Its size is vornd's: the record moves, nothing is sent.
    ptyManager.resizePty(session.id, 120, 40)
    expect(session.cols).toBe(120)
    expect(fake.made('terminal:resize')).toEqual([])

    // Read from vornd's model of the screen.
    fake.output = ['$ make', 'ok']
    expect(await ptyManager.readOutput(session.id, 2)).toEqual(['$ make', 'ok'])
    expect(fake.made('terminal:readOutput').at(-1)).toEqual({ id: session.id, lines: 2 })

    // It exits there, once.
    fake.send('vornd:effect', effect(session.id, 'exit', 9, { code: 4, exitCode: 4 }))
    fake.send('terminal:exit', { id: session.id, exitCode: 4 })
    await until('the exit', () => told(IPC.TERMINAL_EXIT, session.id).length > 0)
    expect(told(IPC.TERMINAL_EXIT, session.id)).toEqual([{ id: session.id, exitCode: 4 }])
    expect(session.shellExitCode).toBe(4)
    expect(ptyManager.hasLivePty(session.id)).toBe(false)
    ptyManager.killPty(session.id)
  })

  it("take an agent's status from vornd unless its hooks report it", async () => {
    const session = await ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p'
    })
    await until('the pid', () => session.pid > 0)
    // The launch line waits for the spawn, then goes in as typing.
    await until('the launch line', () =>
      fake.made('terminal:write').some((w) => w.id === session.id && w.data === 'claude-launch\r')
    )

    fake.send('vornd:effect', effect(session.id, 'status', 3, { status: 2 }))
    await until('waiting', () => session.status === 'waiting')
    fake.send('vornd:effect', effect(session.id, 'status', 4, { status: 1 }))
    await until('running', () => session.status === 'running')

    // Idle after a quiet spell, and running again when it prints with nothing new to say.
    ptyManager.updateSessionStatus(session.id, 'idle')
    fake.send('vornd:activity', { id: session.id })
    await until('running again', () => session.status === 'running')

    // Hooks win once they report.
    ptyManager.promoteToHookStatus(session.id)
    fake.send('vornd:effect', effect(session.id, 'status', 5, { status: 3 }))
    fake.send('vornd:activity', { id: session.id })
    await new Promise((r) => setTimeout(r, 50))
    expect(session.status).toBe('running')

    ptyManager.killPty(session.id)
    await until('the kill', () =>
      fake.made('vornd:kill').some((k) => k.id === session.id && k.signal === 'hup')
    )
  })

  it("take on a terminal vornd holds from the server's last run", async () => {
    const record = {
      id: 'from-last-run',
      agentType: 'shell' as const,
      projectName: 'p',
      projectPath: '/p',
      status: 'idle' as const,
      createdAt: 1,
      pid: 0,
      displayName: 'Shell 1'
    }
    ptyManager.adoptVornd(record, {
      id: 'from-last-run',
      kind: 'pty',
      pid: 77,
      status: null,
      cwd: effect('from-last-run', 'cwd', 2, { cwd: '/p/src' }),
      exit: null
    })
    const live = ptyManager.getLiveSessions().find((s) => s.id === 'from-last-run')!
    expect(live.pid).toBe(77)
    expect(live.status).toBe('running')
    await until('its cwd', () => live.shellCwd === '/p/src')

    // The server stopping leaves it there.
    const before = fake.made('vornd:kill').length
    ptyManager.killAll()
    expect(fake.made('vornd:kill').length).toBe(before)
    expect(vorndSessions.get('from-last-run')).toBeUndefined()
  })
})

describe('headless agents in vornd', () => {
  it('run on pipes: prompt on stdin then closed, output read, exit told once', async () => {
    const session = await headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'write the tests'
    })
    await until('the pid', () => session.pid > 0)
    const spec = spawned(session.id)!
    expect(spec.piped).toBe(true)
    expect(spec.argv).toEqual(expect.arrayContaining(['/opt/agents/bin/claude', '-p']))
    // The prompt, then the end of input, in that order.
    await until('the end of input', () =>
      fake.made('vornd:closeStdin').some((c) => c.id === session.id)
    )
    const order = fake.calls
      .filter((c) => c.params.id === session.id)
      .map((c) => (c.method === 'terminal:write' ? `write ${c.params.data}` : c.method))
    expect(order.indexOf('write write the tests')).toBeLessThan(order.indexOf('vornd:closeStdin'))
    // Read from the start of its records.
    expect(fake.made('terminal:attach').find((a) => a.id === session.id)?.cursor).toEqual({
      epoch: 7,
      nextRseq: 0,
      nextOffset: 0
    })

    fake.sendOutput(session.id, 7, 0, 'hello ')
    fake.sendOutput(session.id, 7, 1, 'world\n')
    // Told again from the start, after a resync: not read twice.
    fake.sendOutput(session.id, 7, 0, 'hello ')
    fake.send('vornd:effect', effect(session.id, 'exit', 2, { code: 0, exitCode: 0 }))
    fake.send('terminal:exit', { id: session.id, exitCode: 0 })
    await until('the exit', () => told(IPC.HEADLESS_EXIT, session.id).length > 0)
    const data = told(IPC.HEADLESS_DATA, session.id)
      .map((d) => d.data)
      .join('')
    expect(data).toBe('hello world\n')
    expect(told(IPC.HEADLESS_EXIT, session.id)).toEqual([{ id: session.id, exitCode: 0 }])
    expect(session.status).toBe('exited')
  })

  it('are signalled in vornd, and left running when the server stops', async () => {
    const session = await headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'x'
    })
    await until('the pid', () => session.pid > 0)
    headlessManager.killHeadless(session.id)
    await until('the term', () =>
      fake.made('vornd:kill').some((k) => k.id === session.id && k.signal === 'term')
    )
    const before = fake.made('vornd:kill').length
    headlessManager.killAll()
    expect(fake.made('vornd:kill').length).toBe(before)
  })
})

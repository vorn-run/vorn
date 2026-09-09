import { describe, it, expect, vi } from 'vitest'
import type { HeadlessSession, TerminalSession } from '../packages/shared/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
// `serve` is the only command that needs one, and no test here runs it.
vi.mock('../packages/server/src/index', () => ({ startServer: vi.fn() }))

import { runCli, type CliDeps } from '../packages/server/src/cli'
import type { RpcTransport } from '../packages/server/src/cli/transport'

type Handlers = Record<string, (params: unknown) => unknown>

/**
 * A server that answers from a script.
 *
 * Every command is driven through this rather than a socket, which is why the
 * transport is injected in the first place.
 */
function fakeRpc(handlers: Handlers): {
  transport: RpcTransport
  calls: { method: string; params: unknown; timeoutMs?: number }[]
} {
  const calls: { method: string; params: unknown; timeoutMs?: number }[] = []
  const transport = {
    async call(method: string, params: unknown, timeoutMs?: number) {
      calls.push({ method, params, timeoutMs })
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

const terminal = (over: Partial<TerminalSession> = {}): TerminalSession =>
  ({
    id: 'c3f1a2e8-1111-2222-3333-444455556666',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/repo/vorn',
    status: 'running',
    createdAt: Date.now(),
    pid: 1234,
    branch: 'main',
    ...over
  }) as TerminalSession

const headless = (over: Partial<HeadlessSession> = {}): HeadlessSession =>
  ({
    id: '9b4d0117-aaaa-bbbb-cccc-ddddeeeeffff',
    pid: 4321,
    agentType: 'codex',
    projectName: 'vorn',
    projectPath: '/repo/vorn',
    status: 'running',
    startedAt: Date.now(),
    ...over
  }) as HeadlessSession

describe('session dispatch', () => {
  it('treats a bare noun as a usage error, on stderr', async () => {
    const io = capture(fakeRpc({}).transport)
    expect(await runCli(['session'], io)).toBe(2)
    expect(io.err()).toContain('vorn session start')
    expect(io.out()).toBe('')
  })

  it('reports an unknown verb', async () => {
    const io = capture(fakeRpc({}).transport)
    expect(await runCli(['session', 'dance'], io)).toBe(2)
    expect(io.err()).toContain('unknown session command "dance"')
  })

  it('gives up with its own code when no server could be reached', async () => {
    const io = capture(fakeRpc({}).transport)
    io.ensureServer = async () => false
    expect(await runCli(['session', 'list'], io)).toBe(4)
  })
})

describe('session list', () => {
  it('shows running sessions of both kinds, short ids, no colour when piped', async () => {
    const { transport } = fakeRpc({
      'terminal:listActive': () => [terminal()],
      'headless:list': () => [headless(), headless({ id: 'gone', status: 'exited' })]
    })
    const io = capture(transport)

    expect(await runCli(['session', 'list'], io)).toBe(0)
    const out = io.out()
    expect(out).toContain('ID        AGENT')
    expect(out).toContain('c3f1a2e8')
    expect(out).toContain('9b4d0117')
    expect(out).not.toContain('gone')
    expect(out).not.toContain(String.fromCharCode(27))
  })

  it('prints what the server said when asked for json', async () => {
    const sessions = [terminal()]
    const { transport } = fakeRpc({
      'terminal:listActive': () => sessions,
      'headless:list': () => []
    })
    const io = capture(transport)

    expect(await runCli(['session', 'list', '--json'], io)).toBe(0)
    expect(JSON.parse(io.out())).toEqual(sessions)
  })

  it('lists past sessions with --recent, from the server that keeps them', async () => {
    const { transport, calls } = fakeRpc({
      'sessions:getRecent': () => [
        {
          sessionId: 'aa11bb22-cccc-dddd-eeee-ffff00001111',
          agentType: 'claude',
          display: 'fix the failing test',
          projectPath: '/repo/vorn',
          timestamp: Date.now() - 3_600_000,
          activityCount: 12,
          activityLabel: '12 turns',
          canResumeExact: true
        }
      ]
    })
    const io = capture(transport)

    expect(await runCli(['session', 'list', '--recent'], io)).toBe(0)
    expect(calls.at(-1)?.method).toBe('sessions:getRecent')
    expect(io.out()).toContain('aa11bb22')
    expect(io.out()).toContain('vorn')
    expect(io.out()).toContain('1h ago')
    expect(io.out()).toContain('12 turns')
  })

  it('says so when nothing recent is kept either', async () => {
    const { transport } = fakeRpc({ 'sessions:getRecent': () => [] })
    const io = capture(transport)

    expect(await runCli(['session', 'list', '--recent'], io)).toBe(0)
    expect(io.out()).toBe('')
    expect(io.err()).toContain('No recent sessions.')
  })

  it('filters by project', async () => {
    const { transport } = fakeRpc({
      'terminal:listActive': () => [terminal(), terminal({ id: 'other', projectName: 'website' })],
      'headless:list': () => []
    })
    const io = capture(transport)

    await runCli(['session', 'list', '--project', 'website'], io)
    expect(io.out()).toContain('website')
    expect(io.out()).not.toContain('c3f1a2e8')
  })

  it('gives every call the ceiling --timeout named', async () => {
    const { transport, calls } = fakeRpc({
      'terminal:listActive': () => [],
      'headless:list': () => []
    })
    const io = capture(transport)

    await runCli(['session', 'list', '--timeout', '500'], io)
    expect(calls.every((c) => c.timeoutMs === 500)).toBe(true)
  })

  it('says nothing on stdout when there is nothing running', async () => {
    const { transport } = fakeRpc({
      'terminal:listActive': () => [],
      'headless:list': () => []
    })
    const io = capture(transport)

    expect(await runCli(['session', 'list'], io)).toBe(0)
    expect(io.out()).toBe('')
    expect(io.err()).toContain('No sessions running.')
  })
})

describe('session start', () => {
  it('registers a project Vorn has not seen, then launches in it', async () => {
    const saved: unknown[] = []
    const { transport, calls } = fakeRpc({
      'config:load': () => ({ projects: [] }),
      'config:save': (params) => {
        saved.push(params)
      },
      'terminal:create': () => terminal()
    })
    const io = capture(transport)

    const code = await runCli(
      ['session', 'start', '--agent', 'claude', '--path', '/repo/vorn', '--prompt', 'xsts'],
      io
    )

    expect(code).toBe(0)
    expect(saved).toEqual([
      { projects: [{ name: 'vorn', path: '/repo/vorn', preferredAgents: ['claude'] }] }
    ])
    expect(calls.at(-1)).toEqual({
      method: 'terminal:create',
      params: {
        agentType: 'claude',
        projectName: 'vorn',
        projectPath: '/repo/vorn',
        initialPrompt: 'xsts'
      }
    })
    expect(io.out()).toContain('session  c3f1a2e8-1111-2222-3333-444455556666')
  })

  it('uses the project a known path already belongs to, and saves nothing', async () => {
    const { transport, calls } = fakeRpc({
      'config:load': () => ({
        projects: [{ name: 'Vorn', path: '/repo/vorn', preferredAgents: ['claude'] }]
      }),
      'terminal:create': () => terminal()
    })
    const io = capture(transport)

    await runCli(['session', 'start', '--agent', 'codex', '--path', '/repo/vorn'], io)
    expect(calls.some((c) => c.method === 'config:save')).toBe(false)
    expect(calls.at(-1)?.params).toMatchObject({ projectName: 'Vorn', agentType: 'codex' })
  })

  it('finds a project by name, from anywhere', async () => {
    const { transport, calls } = fakeRpc({
      'config:load': () => ({
        projects: [{ name: 'website', path: '/repo/website', preferredAgents: [] }]
      }),
      'terminal:create': () => terminal()
    })
    const io = capture(transport)

    await runCli(['session', 'start', '--agent', 'claude', '--project', 'website'], io)
    expect(calls.at(-1)?.params).toMatchObject({
      projectName: 'website',
      projectPath: '/repo/website'
    })
    expect(calls.some((c) => c.method === 'config:save')).toBe(false)
  })

  it('sends a headless launch to the other method', async () => {
    const { transport, calls } = fakeRpc({
      'config:load': () => ({
        projects: [{ name: 'vorn', path: '/repo/vorn', preferredAgents: [] }]
      }),
      'headless:create': () => headless()
    })
    const io = capture(transport)

    await runCli(
      ['session', 'start', '--agent', 'claude', '--path', '/repo/vorn', '--headless'],
      io
    )
    expect(calls.at(-1)?.method).toBe('headless:create')
  })

  it('needs an agent, and refuses one it does not have', async () => {
    const io = capture(fakeRpc({}).transport)
    expect(await runCli(['session', 'start'], io)).toBe(2)
    expect(io.err()).toContain('needs --agent')

    const io2 = capture(fakeRpc({}).transport)
    expect(await runCli(['session', 'start', '--agent', 'hal'], io2)).toBe(2)
    expect(io2.err()).toContain('unknown agent "hal"')
  })
})

describe('addressing a session', () => {
  it('accepts the eight characters a list prints', async () => {
    const { transport, calls } = fakeRpc({
      'terminal:listActive': () => [terminal()],
      'headless:list': () => [],
      'terminal:readOutput': () => ['first line', 'second line']
    })
    const io = capture(transport)

    expect(await runCli(['session', 'logs', 'c3f1a2e8'], io)).toBe(0)
    expect(calls.at(-1)?.params).toEqual({
      id: 'c3f1a2e8-1111-2222-3333-444455556666',
      lines: undefined
    })
    expect(io.out()).toBe('first line\nsecond line\n')
  })

  it('says so on stderr when a session has kept nothing, leaving stdout empty', async () => {
    const { transport } = fakeRpc({
      'terminal:listActive': () => [terminal()],
      'headless:list': () => [],
      'terminal:readOutput': () => []
    })
    const io = capture(transport)

    expect(await runCli(['session', 'logs', 'c3f1a2e8'], io)).toBe(0)
    expect(io.out()).toBe('')
    expect(io.err()).toContain('Nothing kept for c3f1a2e8')
  })

  it('refuses a prefix that names more than one', async () => {
    const { transport } = fakeRpc({
      'terminal:listActive': () => [terminal({ id: 'aa-1' })],
      'headless:list': () => [headless({ id: 'aa-2' })]
    })
    const io = capture(transport)

    expect(await runCli(['session', 'kill', 'aa'], io)).toBe(1)
    expect(io.err()).toContain('matches 2 sessions')
  })

  it('addresses a headless session too, and kills it as one', async () => {
    const { transport, calls } = fakeRpc({
      'terminal:listActive': () => [],
      'headless:list': () => [headless()],
      'headless:kill': () => undefined
    })
    const io = capture(transport)

    expect(await runCli(['session', 'kill', '9b4d0117'], io)).toBe(0)
    expect(calls.at(-1)).toEqual({
      method: 'headless:kill',
      params: '9b4d0117-aaaa-bbbb-cccc-ddddeeeeffff',
      timeoutMs: undefined
    })
  })

  it('explains that a headless session has no terminal to read or steer', async () => {
    const handlers = { 'terminal:listActive': () => [], 'headless:list': () => [headless()] }

    const io = capture(fakeRpc(handlers).transport)
    expect(await runCli(['session', 'logs', '9b4d0117'], io)).toBe(1)
    expect(io.err()).toContain('headless session')
    expect(io.err()).toContain('vorn session kill 9b4d0117')

    const io2 = capture(fakeRpc(handlers).transport)
    expect(await runCli(['session', 'send', '9b4d0117', 'hi'], io2)).toBe(1)
    expect(io2.err()).toContain('no terminal to send to')
  })

  it('says which id it could not find', async () => {
    const { transport } = fakeRpc({ 'terminal:listActive': () => [], 'headless:list': () => [] })
    const io = capture(transport)

    expect(await runCli(['session', 'logs', 'nope'], io)).toBe(1)
    expect(io.err()).toContain('no session matches "nope"')
  })
})

describe('session send', () => {
  it('submits the text, and leaves it alone with --raw', async () => {
    const { transport, calls } = fakeRpc({
      'terminal:listActive': () => [terminal()],
      'headless:list': () => []
    })
    const io = capture(transport)

    await runCli(['session', 'send', 'c3f1a2e8', 'run the tests'], io)
    expect(calls.at(-1)?.params).toEqual({
      id: 'c3f1a2e8-1111-2222-3333-444455556666',
      data: 'run the tests\r'
    })

    const io2 = capture(transport)
    await runCli(['session', 'send', 'c3f1a2e8', 'q', '--raw'], io2)
    expect(calls.at(-1)?.params).toMatchObject({ data: 'q' })
  })

  it('confirms on stderr, so stdout stays empty', async () => {
    const { transport } = fakeRpc({
      'terminal:listActive': () => [terminal()],
      'headless:list': () => []
    })
    const io = capture(transport)

    await runCli(['session', 'send', 'c3f1a2e8', 'hello'], io)
    expect(io.out()).toBe('')
    expect(io.err()).toContain('Sent to c3f1a2e8')
  })
})

import { describe, it, expect, beforeAll, afterAll, vi } from 'vitest'
import WebSocket from 'ws'
import type { RpcResponse } from '@vornrun/shared/protocol'

const TEST_CREDENTIAL = 'integration-test-credential'

/** Presented on the upgrade, the way the desktop bridge and MCP do. */
function authOptions(): { headers: Record<string, string> } {
  return { headers: { Authorization: `Bearer ${TEST_CREDENTIAL}` } }
}

// Mock native modules that require compilation
vi.mock('node-pty', () => ({
  default: { spawn: vi.fn() },
  spawn: vi.fn()
}))

/**
 * Booting a server probes Tailscale, and the probe is a real process.
 *
 * `startServer` calls `refreshTrustedOrigins`, which `execFile`s the Tailscale
 * binary. On a machine with Tailscale.app installed that spawns a helper which
 * does not reliably die when the ten-second timeout fires, so each run of this
 * file leaves one behind and a few full-suite runs leave a pile. Nothing here
 * is about trusted origins.
 */
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

// Mock database to avoid SQLite dependency
vi.mock('../packages/server/src/database', () => ({
  getDb: vi.fn(),
  closeDatabase: vi.fn(),
  initDatabase: vi.fn(),
  // The single resolved data directory the whole server process reads.
  getDataDir: vi.fn(() => '/tmp/vorn-integration-test'),
  dbGetOwnerUser: vi.fn(() => ({
    id: 'owner-1',
    name: 'test',
    role: 'owner' as const,
    createdAt: new Date().toISOString()
  })),
  // Pairing mints a real device token when one is collected, and minting is
  // the only part of that flow which reaches storage.
  dbInsertDeviceToken: vi.fn(),
  dbListDeviceTokens: vi.fn(() => []),
  dbGetDeviceTokenSecret: vi.fn(),
  dbRevokeDeviceToken: vi.fn(() => true),
  dbTouchDeviceToken: vi.fn(),
  loadFullConfig: vi.fn(() => ({
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 14, theme: 'dark' },
    projects: [],
    workflows: [],
    remoteHosts: [],
    tasks: [],
    workspaces: []
  })),
  saveFullConfig: vi.fn(),
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
  listWorkflowRunsByTask: vi.fn(() => []),
  updateWorkflowRunStatus: vi.fn(),
  dbReleaseConnectorInboxLeases: vi.fn(),
  dbCountActiveConnectorInboxLeases: vi.fn(() => 0),
  dbClaimConnectorInbox: vi.fn(() => []),
  dbGetWorkflowRunByConnectorInboxId: vi.fn(() => null),
  loadWorkspaces: vi.fn(() => [])
}))

let serverPort: number
let serverClose: () => Promise<void>

async function sendRpc(
  ws: WebSocket,
  id: number,
  method: string,
  params?: unknown
): Promise<RpcResponse> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`Timeout: ${method}`)), 5000)
    const handler = (raw: WebSocket.RawData) => {
      const msg = JSON.parse(raw.toString()) as RpcResponse
      if (msg.id === id) {
        ws.off('message', handler)
        clearTimeout(timeout)
        resolve(msg)
      }
    }
    ws.on('message', handler)
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
}

describe('server integration', () => {
  beforeAll(async () => {
    // Dynamic import to let mocks take effect
    // The credential the desktop hands its server at spawn. Set before startServer
    // so the real boundary is exercised rather than bypassed.
    process.env.SECRET_VORN_BOOTSTRAP_TOKEN = TEST_CREDENTIAL

    const { startServer } = await import('../packages/server/src/index')

    // Suppress stdout port message during tests
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = (() => true) as typeof process.stdout.write

    try {
      const { app, port } = await startServer({ port: 0 })
      serverPort = port
      serverClose = async () => {
        await app.close()
      }
    } finally {
      process.stdout.write = origWrite
    }
  }, 15000)

  afterAll(async () => {
    delete process.env.SECRET_VORN_BOOTSTRAP_TOKEN
    await serverClose()
  })

  it('health endpoint responds', async () => {
    const res = await fetch(`http://127.0.0.1:${serverPort}/health`)
    expect(res.ok).toBe(true)
    const body = await res.json()
    expect(body.status).toBe('ok')
  })

  it('WebSocket connects', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`, authOptions())
    await new Promise<void>((resolve, reject) => {
      ws.on('open', resolve)
      ws.on('error', reject)
    })
    expect(ws.readyState).toBe(WebSocket.OPEN)
    ws.close()
  })

  it('JSON-RPC request returns result', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`, authOptions())
    await new Promise<void>((r) => ws.on('open', r))

    const res = await sendRpc(ws, 1, 'config:load')
    expect(res.jsonrpc).toBe('2.0')
    expect(res.id).toBe(1)
    expect(res.result).toBeDefined()
    expect((res.result as { defaults: unknown }).defaults).toBeDefined()

    ws.close()
  })

  it('unknown method returns error', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`, authOptions())
    await new Promise<void>((r) => ws.on('open', r))

    const res = await sendRpc(ws, 2, 'nonexistent:method')
    expect(res.error).toBeDefined()
    expect(res.error?.code).toBe(-32601)
    expect(res.error?.message).toContain('Method not found')

    ws.close()
  })

  /**
   * The real route, over a real socket. Browsers let any page open a connection to
   * a loopback server, so these two are what stand between a visited website and
   * a shell on this machine.
   */
  describe('the socket boundary', () => {
    it('refuses an unauthenticated socket instead of serving it', async () => {
      const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`)
      const closed = new Promise<number>((resolve) => ws.on('close', resolve))
      await new Promise<void>((r) => ws.on('open', r))

      ws.send(JSON.stringify({ jsonrpc: '2.0', id: 99, method: 'config:load' }))

      expect(await closed).toBe(4001)
    })

    it('greets an unauthenticated socket, so a browser knows to send a token', async () => {
      const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`)
      const first = await new Promise<string>((resolve) => {
        ws.on('message', (raw) => resolve(raw.toString()))
      })

      expect(JSON.parse(first).method).toBe('server:hello')
      expect(JSON.parse(first).params.capabilities).toEqual({ auth: 1, subscribe: 1 })
      ws.close()
    })

    it('accepts a browser-shaped socket that authenticates by message', async () => {
      const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`)
      await new Promise<void>((r) => ws.on('open', r))

      const ok = new Promise<void>((resolve) => {
        ws.on('message', (raw) => {
          if (JSON.parse(raw.toString()).method === 'auth:ok') resolve()
        })
      })
      ws.send(
        JSON.stringify({
          jsonrpc: '2.0',
          method: 'auth:authenticate',
          params: { token: TEST_CREDENTIAL }
        })
      )
      await ok

      const res = await sendRpc(ws, 50, 'config:load')
      expect(res.result).toBeDefined()
      ws.close()
    })

    it('refuses the upgrade outright from a foreign origin', async () => {
      const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`, {
        headers: { ...authOptions().headers, Origin: 'https://evil.example' }
      })

      // Rejected at the HTTP upgrade, before a socket exists — a valid credential
      // does not save it, because a hostile page could hold one it phished.
      const status = await new Promise<number>((resolve, reject) => {
        ws.on('unexpected-response', (_req, res) => resolve(res.statusCode ?? 0))
        ws.on('open', () => reject(new Error('upgrade should have been refused')))
        ws.on('error', () => {
          /* the refusal surfaces as unexpected-response */
        })
      })
      expect(status).toBe(403)
    })

    it('accepts its own origin, which is where the web client is served from', async () => {
      const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`, {
        headers: { ...authOptions().headers, Origin: `http://127.0.0.1:${serverPort}` }
      })
      await new Promise<void>((resolve, reject) => {
        ws.on('open', resolve)
        ws.on('error', reject)
      })
      expect(ws.readyState).toBe(WebSocket.OPEN)
      ws.close()
    })
  })

  /**
   * Pairing, over the routes a phone with no credential actually uses. The
   * point of these is what they refuse: a token may only ever leave here after
   * a person approved it on the machine being paired to.
   */
  describe('the pairing routes', () => {
    const post = async (
      path: string,
      body: unknown
    ): Promise<{ status: number; json: Record<string, unknown> }> => {
      const res = await fetch(`http://127.0.0.1:${serverPort}${path}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body)
      })
      return { status: res.status, json: await res.json().catch(() => null) }
    }

    /** Ask the server, over an authenticated socket, for a code to show. */
    const startPairing = async (): Promise<string> => {
      const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`, authOptions())
      await new Promise<void>((r) => ws.on('open', r))
      const reply = await sendRpc(ws, 501, 'pairing:start')
      const { code } = reply.result as { code: string }
      ws.close()
      return code
    }

    const decide = async (method: string, requestId: string): Promise<void> => {
      const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`, authOptions())
      await new Promise<void>((r) => ws.on('open', r))
      await sendRpc(ws, 502, method, { requestId })
      ws.close()
    }

    it('refuses a request that is not JSON, so a form post cannot reach it', async () => {
      const res = await fetch(`http://127.0.0.1:${serverPort}/api/pair/redeem`, {
        method: 'POST',
        headers: { 'content-type': 'application/x-www-form-urlencoded' },
        body: 'code=AAAA-AAAA'
      })

      expect(res.status).toBe(415)
    })

    it('refuses a code nobody is showing', async () => {
      const { status } = await post('/api/pair/redeem', {
        code: 'AAAA-AAAA',
        deviceName: 'iPhone'
      })

      expect(status).toBe(400)
    })

    it('hands over no token while the request is still waiting', async () => {
      const code = await startPairing()
      const { json } = await post('/api/pair/redeem', { code, deviceName: 'iPhone' })

      const polled = await post('/api/pair/poll', { requestId: json.requestId })

      expect(polled.json).toEqual({ status: 'pending' })
    })

    it('hands over a token once a person approved it', async () => {
      const code = await startPairing()
      const { json } = await post('/api/pair/redeem', { code, deviceName: 'iPhone' })
      await decide('pairing:approve', json.requestId)

      const polled = await post('/api/pair/poll', { requestId: json.requestId })

      expect(polled.json.status).toBe('approved')
      expect(polled.json.token).toMatch(/^vorn_/)
      expect(typeof polled.json.name).toBe('string')
    })

    it('hands over nothing once a person denied it', async () => {
      const code = await startPairing()
      const { json } = await post('/api/pair/redeem', { code, deviceName: 'iPhone' })
      await decide('pairing:deny', json.requestId)

      const polled = await post('/api/pair/poll', { requestId: json.requestId })

      expect(polled.json).toEqual({ status: 'denied' })
    })

    it('lets the token be collected once and not again', async () => {
      const code = await startPairing()
      const { json } = await post('/api/pair/redeem', { code, deviceName: 'iPhone' })
      await decide('pairing:approve', json.requestId)
      await post('/api/pair/poll', { requestId: json.requestId })

      const second = await post('/api/pair/poll', { requestId: json.requestId })

      expect(second.json).toEqual({ status: 'expired' })
    })

    it('will not start pairing for an unauthenticated socket', async () => {
      const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`)
      const closed = new Promise<number>((resolve) => ws.on('close', resolve))
      await new Promise<void>((r) => ws.on('open', r))

      ws.send(JSON.stringify({ jsonrpc: '2.0', id: 77, method: 'pairing:start' }))

      expect(await closed).toBe(4001)
    })
  })

  /**
   * A phone connects over cellular and renders no terminals, but the registry
   * used to hand every socket every notification — including every byte of every
   * PTY on the machine.
   */
  describe('a socket that asked for less', () => {
    /**
     * Collect notifications until the server has plainly finished sending.
     *
     * Idle-based rather than a fixed sleep: the assertion is partly that certain
     * frames never arrive, and a fixed wait either ends before a slow one lands —
     * passing for the wrong reason on a loaded machine — or pads every run to be
     * sure. Each frame restarts the clock, so this settles as fast as the server
     * allows and still waits when the server is slow.
     */
    const HANDSHAKE_FRAMES = new Set(['server:hello', 'server:identity'])

    async function notificationsFor(query: string): Promise<string[]> {
      const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws${query}`, authOptions())
      const methods: string[] = []
      let idle: ReturnType<typeof setTimeout>
      const settled = new Promise<void>((resolve) => {
        const restart = (): void => {
          clearTimeout(idle)
          idle = setTimeout(resolve, 60)
        }
        ws.on('message', (raw) => {
          const frame = JSON.parse(raw.toString())
          // The handshake frames are sent directly on the socket, outside the
          // broadcast set this test is measuring, so neither is a notification
          // any topic filter was ever asked about.
          if (frame.method && !HANDSHAKE_FRAMES.has(frame.method)) methods.push(frame.method)
          restart()
        })
        restart()
      })
      await new Promise<void>((r) => ws.on('open', r))

      const { clientRegistry } = await import('../packages/server/src/broadcast')
      clientRegistry.broadcast('terminal:data', { id: 'a', data: 'x' })
      clientRegistry.broadcast('session:updated', { id: 'a' })

      await settled
      clearTimeout(idle)
      ws.close()
      return methods
    }

    it('still receives everything when it asks for nothing', async () => {
      expect(await notificationsFor('')).toEqual(['terminal:data', 'session:updated'])
    })

    it('receives only what the URL asked for', async () => {
      // On the URL rather than in a later frame: by the time a frame could arrive
      // the socket is already in the broadcast set, and that gap repeats on every
      // reconnect.
      expect(await notificationsFor('?topics=session:*')).toEqual(['session:updated'])
    })
  })

  it('fire-and-forget notification does not crash', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`, authOptions())
    await new Promise<void>((r) => ws.on('open', r))

    // Send notification (no id — server should not respond)
    ws.send(
      JSON.stringify({
        jsonrpc: '2.0',
        method: 'terminal:write',
        params: { id: 'nonexistent', data: 'hello' }
      })
    )

    // Wait a bit to confirm no crash
    await new Promise((r) => setTimeout(r, 200))
    expect(ws.readyState).toBe(WebSocket.OPEN)

    ws.close()
  })
})

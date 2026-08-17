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
      expect(JSON.parse(first).params.capabilities).toEqual({ auth: 1 })
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

import { describe, it, expect, beforeAll, afterAll, vi } from 'vitest'
import WebSocket from 'ws'
import type { RpcResponse } from '@vornrun/shared/protocol'

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
    await serverClose()
  })

  it('health endpoint responds', async () => {
    const res = await fetch(`http://127.0.0.1:${serverPort}/health`)
    expect(res.ok).toBe(true)
    const body = await res.json()
    expect(body.status).toBe('ok')
  })

  it('WebSocket connects', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`)
    await new Promise<void>((resolve, reject) => {
      ws.on('open', resolve)
      ws.on('error', reject)
    })
    expect(ws.readyState).toBe(WebSocket.OPEN)
    ws.close()
  })

  it('JSON-RPC request returns result', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`)
    await new Promise<void>((r) => ws.on('open', r))

    const res = await sendRpc(ws, 1, 'config:load')
    expect(res.jsonrpc).toBe('2.0')
    expect(res.id).toBe(1)
    expect(res.result).toBeDefined()
    expect((res.result as { defaults: unknown }).defaults).toBeDefined()

    ws.close()
  })

  it('unknown method returns error', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`)
    await new Promise<void>((r) => ws.on('open', r))

    const res = await sendRpc(ws, 2, 'nonexistent:method')
    expect(res.error).toBeDefined()
    expect(res.error?.code).toBe(-32601)
    expect(res.error?.message).toContain('Method not found')

    ws.close()
  })

  it('fire-and-forget notification does not crash', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${serverPort}/ws`)
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

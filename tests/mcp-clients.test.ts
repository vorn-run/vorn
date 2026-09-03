import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SourceConnection } from '../packages/shared/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const transportInstances: MockTransport[] = []
const clientConnect = vi.fn()
const clientClose = vi.fn()

class MockTransport {
  readonly opts: unknown
  closed = false
  onclose: (() => void) | undefined
  onerror: ((err: unknown) => void) | undefined

  constructor(opts: unknown) {
    this.opts = opts
    transportInstances.push(this)
  }

  async close(): Promise<void> {
    this.closed = true
  }
}

vi.mock('@modelcontextprotocol/sdk/client/index.js', () => ({
  Client: class {
    constructor(_info: unknown, _caps: unknown) {}
    connect = clientConnect
    close = clientClose
  }
}))

vi.mock('@modelcontextprotocol/sdk/client/stdio.js', () => ({
  StdioClientTransport: MockTransport
}))

const importClients = async () => await import('../packages/server/src/connectors/mcp-clients')
const importDecrypted = async () =>
  await import('../packages/server/src/connectors/decrypted-creds')

beforeEach(async () => {
  vi.resetModules()
  transportInstances.length = 0
  clientConnect.mockReset()
  clientClose.mockReset()
  // Touch the decrypted-creds module so each test starts with an empty store.
  ;(await importDecrypted()).clearDecryptedCreds('placeholder')
})

function conn(filters: Record<string, unknown>): SourceConnection {
  return {
    id: 'c1',
    connectorId: 'mcp',
    name: 'Test',
    filters,
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-04-24T00:00:00Z'
  }
}

describe('getOrStartClient', () => {
  it('spawns the child process with the parsed args + env on first call', async () => {
    clientConnect.mockResolvedValue(undefined)
    const { getOrStartClient } = await importClients()
    await getOrStartClient(
      conn({
        command: 'npx',
        args: '["-y","filesystem","/tmp"]',
        env: '{"FOO":"bar"}'
      })
    )
    expect(transportInstances).toHaveLength(1)
    const opts = transportInstances[0].opts as {
      command: string
      args: string[]
      env: Record<string, string>
    }
    expect(opts.command).toBe('npx')
    expect(opts.args).toEqual(['-y', 'filesystem', '/tmp'])
    expect(opts.env.FOO).toBe('bar')
  })

  it('reuses the cached client on subsequent calls', async () => {
    clientConnect.mockResolvedValue(undefined)
    const { getOrStartClient } = await importClients()
    const c = conn({ command: 'echo', args: '[]', env: '{}' })
    await getOrStartClient(c)
    await getOrStartClient(c)
    expect(transportInstances).toHaveLength(1)
  })

  it('does not race two spawns when concurrent callers hit a cache miss', async () => {
    let resolveConnect: (() => void) | undefined
    clientConnect.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveConnect = resolve
        })
    )
    const { getOrStartClient } = await importClients()
    const c = conn({ command: 'echo', args: '[]', env: '{}' })
    // Two parallel callers — both should share the same in-flight startup
    // and end up using the same single transport.
    const p1 = getOrStartClient(c)
    const p2 = getOrStartClient(c)
    await vi.waitFor(() => expect(transportInstances).toHaveLength(1))
    resolveConnect?.()
    const [a, b] = await Promise.all([p1, p2])
    expect(a).toBe(b)
    expect(transportInstances).toHaveLength(1)
  })

  it('merges decrypted secretEnv on top of plaintext env', async () => {
    clientConnect.mockResolvedValue(undefined)
    const { setDecryptedCreds } = await importDecrypted()
    setDecryptedCreds('c1', { secretEnv: '{"TOKEN":"shh"}' })
    const { getOrStartClient } = await importClients()
    await getOrStartClient(conn({ command: 'echo', args: '[]', env: '{"FOO":"public"}' }))
    const opts = transportInstances[0].opts as { env: Record<string, string> }
    expect(opts.env.TOKEN).toBe('shh')
    expect(opts.env.FOO).toBe('public')
  })

  it('treats malformed args/env JSON as empty', async () => {
    clientConnect.mockResolvedValue(undefined)
    const { getOrStartClient } = await importClients()
    await getOrStartClient(conn({ command: 'echo', args: 'not-json', env: 'also-not-json' }))
    const opts = transportInstances[0].opts as { args: string[]; env: Record<string, string> }
    expect(opts.args).toEqual([])
    // env still inherits process.env, so check there's no parsed key from the bad input.
    expect(opts.env.FOO).toBeUndefined()
  })

  it('rejects when no command is provided', async () => {
    const { getOrStartClient } = await importClients()
    await expect(getOrStartClient(conn({ command: '', args: '[]', env: '{}' }))).rejects.toThrow(
      /missing a command/
    )
  })

  it('clears the cache entry when the transport closes', async () => {
    clientConnect.mockResolvedValue(undefined)
    const { getOrStartClient, hasClient } = await importClients()
    const c = conn({ command: 'echo', args: '[]', env: '{}' })
    await getOrStartClient(c)
    expect(hasClient('c1')).toBe(true)
    transportInstances[0].onclose?.()
    expect(hasClient('c1')).toBe(false)
  })
})

describe('stopClient / stopAllClients', () => {
  it('removes the entry and calls client.close', async () => {
    clientConnect.mockResolvedValue(undefined)
    clientClose.mockResolvedValue(undefined)
    const { getOrStartClient, stopClient, hasClient } = await importClients()
    await getOrStartClient(conn({ command: 'echo', args: '[]', env: '{}' }))
    await stopClient('c1')
    expect(hasClient('c1')).toBe(false)
    expect(clientClose).toHaveBeenCalled()
  })

  it('stopAllClients tears down every cached entry', async () => {
    clientConnect.mockResolvedValue(undefined)
    clientClose.mockResolvedValue(undefined)
    const { getOrStartClient, stopAllClients, hasClient } = await importClients()
    const a = conn({ command: 'echo', args: '[]', env: '{}' })
    await getOrStartClient(a)
    await getOrStartClient({ ...a, id: 'c2' })
    await stopAllClients()
    expect(hasClient('c1')).toBe(false)
    expect(hasClient('c2')).toBe(false)
  })
})

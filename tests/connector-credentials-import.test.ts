import { describe, it, expect, vi } from 'vitest'

vi.mock('electron', () => ({
  safeStorage: { isEncryptionAvailable: () => true, decryptString: (b: Buffer) => b.toString() }
}))
vi.mock('../src/main/logger', () => ({ default: { info: vi.fn(), warn: vi.fn() } }))

const { IN_VAULT, sealedSecrets, installConnectorCredentialsImport } =
  await import('../src/main/connector-credentials-import')

const connectors = [
  {
    id: 'http',
    manifest: {
      auth: [
        { key: 'secret', label: 'S', type: 'password' },
        { key: 'baseUrl', label: 'B', type: 'text' }
      ]
    }
  },
  { id: 'mcp', manifest: { auth: [{ key: 'secretEnv', label: 'E', type: 'password' }] } }
]

const conn = (id: string, connectorId: string, filters: Record<string, unknown>) =>
  ({
    id,
    connectorId,
    name: id,
    filters,
    syncIntervalMinutes: 0,
    statusMapping: {},
    createdAt: ''
  }) as never

describe('the secrets this app sealed before vornd kept them', () => {
  it('decrypts each sealed secret field once, and skips what vornd already holds', () => {
    const decrypt = (sealed: string) => (sealed === 'broken' ? undefined : `plain:${sealed}`)
    const sealed = sealedSecrets(
      [
        conn('a', 'http', { secret: 'c1', baseUrl: 'https://x' }),
        conn('b', 'mcp', { secretEnv: IN_VAULT }),
        conn('c', 'mcp', { secretEnv: 'broken' }),
        conn('d', 'unknown', { secret: 'c2' })
      ],
      connectors as never,
      decrypt
    )
    expect(sealed).toEqual({ a: { secret: 'plain:c1' } })
  })

  it('hands vornd what it found, and nothing when there is nothing', async () => {
    vi.useFakeTimers()
    try {
      const asked: Array<[string, unknown]> = []
      const bridge = {
        request: vi.fn(async (method: string, params: unknown) => {
          asked.push([method, params])
          if (method === 'connection:list')
            return [conn('a', 'http', { secret: Buffer.from('k').toString('base64') })]
          if (method === 'connector:list') return connectors
          return { connections: 1 }
        })
      }
      installConnectorCredentialsImport(bridge as never)
      await vi.advanceTimersByTimeAsync(600)
      expect(asked.at(-1)).toEqual(['credentials:import', { connections: { a: { secret: 'k' } } }])
    } finally {
      vi.useRealTimers()
    }
  })
})

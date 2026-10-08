import { describe, it, expect, vi } from 'vitest'

vi.mock('electron', () => ({
  safeStorage: { isEncryptionAvailable: () => true, decryptString: (b: Buffer) => b.toString() }
}))
vi.mock('../src/main/logger', () => ({ default: { info: vi.fn(), warn: vi.fn() } }))

const { IN_VAULT, sealedById, sealedSecrets, installCredentialsImport } =
  await import('../src/main/credentials-import')

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

  it('decrypts each sealed key or password by id, and skips what vornd already holds', () => {
    const decrypt = (sealed: string) => (sealed === 'broken' ? undefined : `plain:${sealed}`)
    expect(
      sealedById(
        [
          ['k1', 'c1'],
          ['k2', IN_VAULT],
          ['k3', 'broken'],
          ['k4', undefined]
        ],
        decrypt
      )
    ).toEqual({ k1: 'plain:c1' })
  })

  it('hands vornd what it found, and nothing when there is nothing', async () => {
    const sealed = (text: string) => Buffer.from(text).toString('base64')
    const run = async (found: boolean): Promise<Array<[string, unknown]>> => {
      const asked: Array<[string, unknown]> = []
      const bridge = {
        request: vi.fn(async (method: string, params: unknown) => {
          asked.push([method, params])
          if (method === 'connection:list')
            return found ? [conn('a', 'http', { secret: sealed('k') })] : []
          if (method === 'connector:list') return connectors
          if (method === 'config:load')
            return {
              remoteHosts: found
                ? [
                    { id: 'h1', encryptedPassword: sealed('pw') },
                    { id: 'h2', encryptedPassword: IN_VAULT }
                  ]
                : []
            }
          if (method === 'credential:listKeys') return found ? [{ id: 's1' }, { id: 's2' }] : []
          if (method === 'credential:getEncryptedKey')
            return params === 's1'
              ? { id: 's1', encryptedPrivateKey: sealed('KEY') }
              : { id: 's2', encryptedPrivateKey: IN_VAULT }
          return { connections: 1, sshKeys: 1, hostPasswords: 1 }
        })
      }
      installCredentialsImport(bridge as never)
      await vi.advanceTimersByTimeAsync(600)
      return asked
    }
    vi.useFakeTimers()
    try {
      expect((await run(true)).at(-1)).toEqual([
        'credentials:import',
        {
          connections: { a: { secret: 'k' } },
          sshKeys: { s1: 'KEY' },
          hostPasswords: { h1: 'pw' }
        }
      ])
      expect((await run(false)).map(([m]) => m)).not.toContain('credentials:import')
    } finally {
      vi.useRealTimers()
    }
  })
})

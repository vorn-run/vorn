import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SdkConnectorAuth, SourceConnection } from '../packages/shared/src/types'

const transportInstances: unknown[] = []
const clientConnect = vi.fn()

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

vi.mock('@modelcontextprotocol/sdk/client/index.js', () => ({
  Client: class {
    connect = clientConnect
    close = vi.fn()
  }
}))

vi.mock('@modelcontextprotocol/sdk/client/stdio.js', () => ({
  StdioClientTransport: class {
    readonly opts: unknown
    onclose: (() => void) | undefined
    onerror: ((err: unknown) => void) | undefined
    constructor(opts: unknown) {
      this.opts = opts
      transportInstances.push(this)
    }
    async close(): Promise<void> {}
  }
}))

// What a packaged connector's child is started with.

const pack = {
  current: undefined as { auth?: SdkConnectorAuth; env?: Array<{ name: string }> } | undefined
}
const decrypted = { current: {} as Record<string, string> }

function connection(filters: Record<string, unknown>): SourceConnection {
  return {
    id: 'conn-1',
    connectorId: 'mcp',
    name: 'Acme',
    filters,
    syncIntervalMinutes: 0,
    statusMapping: {},
    createdAt: new Date().toISOString()
  } as unknown as SourceConnection
}

beforeEach(() => {
  pack.current = undefined
  decrypted.current = {}
  vi.doMock('../packages/server/src/connectors/packs', () => ({
    installedLaunch: () => ({ command: 'node', args: ['/packs/acme/index.js'] }),
    installedPack: () => pack.current
  }))
  vi.doMock('../packages/server/src/connectors/decrypted-creds', () => ({
    getDecryptedCreds: () => decrypted.current
  }))
  vi.doMock('../packages/server/src/process-utils', () => ({
    getSafeEnv: () => ({ PATH: '/usr/bin' }),
    isAbsolutelyStrippedEnvName: (name: string) => name.startsWith('CLAUDE_CODE_'),
    getEnvPassthrough: () => new Set<string>(),
    isSensitiveEnvName: (name: string) => /^(GITHUB_TOKEN|GH_TOKEN|ANTHROPIC_API)/.test(name)
  }))
  vi.resetModules()
})

afterEach(() => {
  vi.unstubAllEnvs()
  vi.resetModules()
})

const load = async (): Promise<typeof import('../packages/server/src/connectors/mcp-clients')> =>
  import('../packages/server/src/connectors/mcp-clients')

/** A pack declares what it reads; the borrow is held to that list. */
const CLI_PACK: { auth: SdkConnectorAuth; env: Array<{ name: string }> } = {
  auth: {
    rung: 'cli',
    probe: { command: 'glab', args: ['auth', 'status'] },
    borrow: { env: ['GITLAB_TOKEN'], tokenArgs: ['auth', 'token'] }
  },
  env: [{ name: 'GITLAB_TOKEN' }]
}

describe('the environment a packaged connector is spawned with', () => {
  it('hands over what the signed-in tool already set', async () => {
    pack.current = CLI_PACK
    vi.stubEnv('GITLAB_TOKEN', 'from-the-shell')
    const { buildSpawnConfig } = await load()
    const spawn = await buildSpawnConfig(connection({ sdkConnectorId: 'acme' }))
    expect(spawn.env.GITLAB_TOKEN).toBe('from-the-shell')
  })

  it('borrows nothing for a connector that asks for a key instead', async () => {
    pack.current = { auth: { rung: 'key', keys: ['token'] }, env: [{ name: 'GITLAB_TOKEN' }] }
    vi.stubEnv('GITLAB_TOKEN', 'from-the-shell')
    const { buildSpawnConfig } = await load()
    const spawn = await buildSpawnConfig(connection({ sdkConnectorId: 'acme' }))
    expect(spawn.env.GITLAB_TOKEN).toBeUndefined()
  })

  it('lets a value entered by hand override what would have been borrowed', async () => {
    pack.current = CLI_PACK
    vi.stubEnv('GITLAB_TOKEN', 'from-the-shell')
    const { buildSpawnConfig } = await load()
    const spawn = await buildSpawnConfig(
      connection({ sdkConnectorId: 'acme', env: JSON.stringify({ GITLAB_TOKEN: 'typed' }) })
    )
    expect(spawn.env.GITLAB_TOKEN).toBe('typed')
  })

  it('keeps a decrypted secret ahead of everything else', async () => {
    pack.current = CLI_PACK
    vi.stubEnv('GITLAB_TOKEN', 'from-the-shell')
    decrypted.current = { secretEnv: JSON.stringify({ GITLAB_TOKEN: 'encrypted' }) }
    const { buildSpawnConfig } = await load()
    const spawn = await buildSpawnConfig(connection({ sdkConnectorId: 'acme' }))
    expect(spawn.env.GITLAB_TOKEN).toBe('encrypted')
  })

  it('asks nothing of a connection that names no packaged connector', async () => {
    vi.stubEnv('GITLAB_TOKEN', 'from-the-shell')
    const { buildSpawnConfig } = await load()
    const spawn = await buildSpawnConfig(connection({ command: 'uvx', args: '["thing"]' }))
    expect(spawn.command).toBe('uvx')
    expect(spawn.env.GITLAB_TOKEN).toBeUndefined()
  })
})

describe('two callers arriving together', () => {
  /** Counts what the borrow costs: one call is one run of the token command. */
  const borrowedSecrets = vi.fn().mockResolvedValue({ GITLAB_TOKEN: 'asked-once' })

  beforeEach(() => {
    transportInstances.length = 0
    clientConnect.mockReset().mockResolvedValue(undefined)
    borrowedSecrets.mockClear()
    vi.doMock('../packages/server/src/connectors/auth-rung', () => ({ borrowedSecrets }))
    vi.resetModules()
  })

  it('asks the borrowed tool for a token once, not once each', async () => {
    pack.current = CLI_PACK
    vi.stubEnv('GITLAB_TOKEN', '')
    const { getOrStartClient } = await load()
    const c = connection({ sdkConnectorId: 'acme' })

    const [first, second] = await Promise.all([getOrStartClient(c), getOrStartClient(c)])

    expect(first).toBe(second)
    expect(borrowedSecrets).toHaveBeenCalledTimes(1)
    expect(transportInstances).toHaveLength(1)
  })
})

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SdkConnectorAuth, SourceConnection } from '../packages/shared/src/types'

/**
 * What a packaged connector's child is started with.
 *
 * The point of the `cli` rung is that nothing is stored: the token is asked
 * for at spawn, so these tests read the environment a spawn would use rather
 * than anything written to the connection.
 */

const pack = {
  current: undefined as { auth?: SdkConnectorAuth } | undefined
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
    describePack: () => pack.current
  }))
  vi.doMock('../packages/server/src/connectors/decrypted-creds', () => ({
    getDecryptedCreds: () => decrypted.current
  }))
  vi.doMock('../packages/server/src/process-utils', () => ({
    getSafeEnv: () => ({ PATH: '/usr/bin' })
  }))
  vi.resetModules()
})

afterEach(() => {
  vi.unstubAllEnvs()
  vi.resetModules()
})

const load = async (): Promise<typeof import('../packages/server/src/connectors/mcp-clients')> =>
  import('../packages/server/src/connectors/mcp-clients')

const CLI_PACK: { auth: SdkConnectorAuth } = {
  auth: {
    rung: 'cli',
    probe: { command: 'glab', args: ['auth', 'status'] },
    borrow: { env: ['GITLAB_TOKEN'], tokenArgs: ['auth', 'token'] }
  }
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
    pack.current = { auth: { rung: 'key', keys: ['token'] } }
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

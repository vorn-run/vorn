import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { SourceConnection } from '@vornrun/shared/types'
import { catalogLaunchSpec, localLaunchSpec } from '../packages/server/src/connectors/catalog'

const temps: string[] = []

function tempDir(): string {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-launch-test-'))
  temps.push(dir)
  return dir
}

/** A connectors checkout with one connector built, as VORN_CONNECTORS_ROOT names. */
function checkoutWith(dirName: string): string {
  const root = tempDir()
  const dist = join(root, 'packages', dirName, 'dist')
  mkdirSync(dist, { recursive: true })
  writeFileSync(join(dist, 'index.js'), '')
  return root
}

function connection(filters: Record<string, unknown>): SourceConnection {
  return {
    id: 'conn-1',
    connectorId: 'mcp',
    name: 'Acme',
    filters,
    syncIntervalMinutes: 0,
    createdAt: new Date().toISOString()
  } as unknown as SourceConnection
}

afterEach(() => {
  while (temps.length > 0) rmSync(temps.pop() as string, { recursive: true, force: true })
  vi.unstubAllEnvs()
  vi.resetModules()
})

describe('localLaunchSpec', () => {
  it('points at a built connector in the checkout', () => {
    const root = checkoutWith('acme')
    expect(localLaunchSpec('acme', root)).toEqual({
      command: 'node',
      args: [join(root, 'packages', 'acme', 'dist', 'index.js')]
    })
  })

  it('ignores a checkout with nothing built and no checkout at all', () => {
    expect(localLaunchSpec('acme', tempDir())).toBeUndefined()
    expect(localLaunchSpec('acme', undefined)).toBeUndefined()
  })
})

describe('catalogLaunchSpec', () => {
  const entry = {
    id: 'acme',
    name: 'Acme',
    description: '',
    packageName: '@vornrun/connector-acme',
    capabilities: []
  } as never

  it('falls back to npx when nothing local is built', () => {
    expect(catalogLaunchSpec(entry, undefined)).toEqual({
      command: 'npx',
      args: ['-y', '@vornrun/connector-acme']
    })
  })

  it('prefers the local build, mapping the package name to its directory', () => {
    const root = checkoutWith('acme')
    expect(catalogLaunchSpec(entry, root).command).toBe('node')
  })
})

describe('resolveLaunch precedence', () => {
  const packsRoot = { current: '' }
  const packsBehavior = { throwUnresolved: false }

  beforeEach(() => {
    packsRoot.current = tempDir()
    packsBehavior.throwUnresolved = false
    vi.doMock('../packages/server/src/connectors/packs', () => ({
      installedLaunch: (id: string) => {
        if (packsBehavior.throwUnresolved) {
          throw new Error('Data directory not resolved. Call initDatabase() first.')
        }
        if (id !== 'acme' || packsRoot.current === '') return undefined
        return { command: 'node', args: [join(packsRoot.current, 'acme', '2.0.0', 'index.js')] }
      }
    }))
  })

  const load = async (): Promise<typeof import('../packages/server/src/connectors/mcp-clients')> =>
    import('../packages/server/src/connectors/mcp-clients')

  it('launches an installed pack over the stored npx command', async () => {
    const { resolveLaunch } = await load()
    const spec = resolveLaunch(
      connection({
        sdkConnectorId: 'acme',
        command: 'npx',
        args: '["-y","@vornrun/connector-acme"]'
      })
    )
    expect(spec.command).toBe('node')
    expect(spec.args[0]).toContain(join('acme', '2.0.0', 'index.js'))
  })

  it('lets a local checkout win over the installed pack', async () => {
    vi.stubEnv('VORN_CONNECTORS_ROOT', checkoutWith('acme'))
    const { resolveLaunch } = await load()
    const spec = resolveLaunch(connection({ sdkConnectorId: 'acme', command: 'npx' }))
    expect(spec.args[0]).toContain(join('packages', 'acme', 'dist', 'index.js'))
  })

  it('keeps an npx-era connection working when nothing is installed', async () => {
    const { resolveLaunch } = await load()
    expect(
      resolveLaunch(connection({ sdkConnectorId: 'other', command: 'npx', args: '["-y","pkg"]' }))
    ).toEqual({ command: 'npx', args: ['-y', 'pkg'] })
  })

  it('uses the stored command for a plain MCP connection with no connector id', async () => {
    const { resolveLaunch } = await load()
    expect(resolveLaunch(connection({ command: 'uvx', args: '["thing"]' }))).toEqual({
      command: 'uvx',
      args: ['thing']
    })
  })

  it('refuses a connection that has neither a pack nor a command', async () => {
    const { resolveLaunch } = await load()
    expect(() => resolveLaunch(connection({}))).toThrow(/missing a command/)
    expect(() => resolveLaunch(connection({ sdkConnectorId: 'other' }))).toThrow(
      /missing a command/
    )
  })

  it('treats an unresolved data directory as nothing installed', async () => {
    packsBehavior.throwUnresolved = true
    const { resolveLaunch } = await load()
    expect(
      resolveLaunch(connection({ sdkConnectorId: 'acme', command: 'npx', args: '[]' }))
    ).toEqual({ command: 'npx', args: [] })
  })
})

describe('stopClientsForConnector', () => {
  const connections: SourceConnection[] = [
    connection({ sdkConnectorId: 'acme' }),
    { ...connection({ sdkConnectorId: 'acme' }), id: 'conn-2' } as SourceConnection,
    { ...connection({ sdkConnectorId: 'other' }), id: 'conn-3' } as SourceConnection,
    { ...connection({ command: 'uvx' }), id: 'conn-4', connectorId: 'github' } as SourceConnection
  ]

  beforeEach(() => {
    vi.doMock('../packages/server/src/database', () => ({
      dbListSourceConnections: () => connections
    }))
    vi.doMock('../packages/server/src/connectors/packs', () => ({
      installedLaunch: () => undefined
    }))
    vi.resetModules()
  })

  it('picks the connections a pack change affects, by connector rather than by row', async () => {
    const { connectionIdsForConnector } =
      await import('../packages/server/src/connectors/mcp-clients')
    expect(connectionIdsForConnector('acme')).toEqual(['conn-1', 'conn-2'])
    expect(connectionIdsForConnector('other')).toEqual(['conn-3'])
    expect(connectionIdsForConnector('github')).toEqual(['conn-4'])
    expect(connectionIdsForConnector('mcp')).toEqual([])
  })

  it('stopping a connector with no live children is a no-op', async () => {
    const { stopClientsForConnector, hasClient } =
      await import('../packages/server/src/connectors/mcp-clients')
    await expect(stopClientsForConnector('acme')).resolves.toBeUndefined()
    expect(hasClient('conn-1')).toBe(false)
  })
})

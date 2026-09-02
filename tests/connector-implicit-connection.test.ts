import { describe, expect, it, vi } from 'vitest'
import type { InstalledConnectorPack, SourceConnection } from '../packages/shared/src/types'
import {
  isImplicit,
  syncImplicitConnection,
  type ImplicitConnectionDeps
} from '../packages/server/src/connectors/implicit-connection'

/**
 * A connector that asks for no sign-in is connected by installing it.
 *
 * What matters is that the connection appears and disappears with the files,
 * and that one somebody made by hand is never withdrawn on their behalf.
 */

const NONE_PACK: InstalledConnectorPack = {
  id: 'echo-bench',
  name: 'Echo Bench',
  version: '1.0.0',
  auth: { rung: 'none' },
  path: '/packs/echo-bench/1.0.0',
  installedAt: 0,
  bytes: 1,
  triggers: [],
  actions: [],
  env: []
}

function connection(filters: Record<string, unknown>): SourceConnection {
  return {
    id: 'existing-1',
    connectorId: 'mcp',
    name: 'Echo Bench',
    filters,
    syncIntervalMinutes: 0,
    statusMapping: {},
    createdAt: new Date().toISOString()
  } as unknown as SourceConnection
}

/** Typed to the hook's own signatures, so a drifting shape fails here first. */
function deps(pack: InstalledConnectorPack | undefined, rows: SourceConnection[] = []) {
  return {
    describe: (): InstalledConnectorPack | undefined => pack,
    list: (): SourceConnection[] => rows,
    create: vi.fn<ImplicitConnectionDeps['create']>(),
    remove: vi.fn<ImplicitConnectionDeps['remove']>(),
    changed: vi.fn<ImplicitConnectionDeps['changed']>()
  } satisfies ImplicitConnectionDeps
}

describe('the connection a connector that needs no sign-in comes with', () => {
  it('is made when such a pack is installed', () => {
    const d = deps(NONE_PACK)
    syncImplicitConnection('echo-bench', d)
    expect(d.create).toHaveBeenCalledTimes(1)
    const made = d.create.mock.calls[0][0]
    expect(made.filters.implicit).toBe(true)
    expect(made.filters.sdkConnectorId).toBe('echo-bench')
    expect(made.name).toBe('Echo Bench')
  })

  it('is not made a second time', () => {
    const d = deps(NONE_PACK, [connection({ sdkConnectorId: 'echo-bench', implicit: true })])
    syncImplicitConnection('echo-bench', d)
    expect(d.create).not.toHaveBeenCalled()
    expect(d.remove).not.toHaveBeenCalled()
  })

  it('is withdrawn when the pack goes', () => {
    const d = deps(undefined, [connection({ sdkConnectorId: 'echo-bench', implicit: true })])
    syncImplicitConnection('echo-bench', d)
    expect(d.remove).toHaveBeenCalledWith('existing-1')
    expect(d.changed).toHaveBeenCalled()
  })

  it('is withdrawn when an update gives the connector something to ask for', () => {
    const asksNow = { ...NONE_PACK, auth: { rung: 'key' as const, keys: ['token'] } }
    const d = deps(asksNow, [connection({ sdkConnectorId: 'echo-bench', implicit: true })])
    syncImplicitConnection('echo-bench', d)
    expect(d.remove).toHaveBeenCalledWith('existing-1')
  })

  it('leaves a connection somebody made by hand alone', () => {
    const d = deps(undefined, [connection({ sdkConnectorId: 'echo-bench' })])
    syncImplicitConnection('echo-bench', d)
    expect(d.remove).not.toHaveBeenCalled()
    expect(d.changed).not.toHaveBeenCalled()
  })

  it('is not made for a connector that asks for a key', () => {
    const d = deps({ ...NONE_PACK, auth: { rung: 'key', keys: ['token'] } })
    syncImplicitConnection('echo-bench', d)
    expect(d.create).not.toHaveBeenCalled()
  })

  it('is not made for a connector that borrows a login', () => {
    const d = deps({
      ...NONE_PACK,
      auth: { rung: 'cli', probe: { command: 'glab', args: ['auth', 'status'] } }
    })
    syncImplicitConnection('echo-bench', d)
    expect(d.create).not.toHaveBeenCalled()
  })

  it('does nothing at all before there is anywhere to look', () => {
    const d: ImplicitConnectionDeps = {
      describe: (): InstalledConnectorPack | undefined => {
        throw new Error('Data directory not resolved. Call initDatabase() first.')
      },
      list: (): SourceConnection[] => [],
      create: vi.fn<ImplicitConnectionDeps['create']>(),
      remove: vi.fn<ImplicitConnectionDeps['remove']>(),
      changed: vi.fn<ImplicitConnectionDeps['changed']>()
    }
    expect(() => syncImplicitConnection('echo-bench', d)).not.toThrow()
  })
})

describe('telling the app apart from a person', () => {
  it('knows which connection it made', () => {
    expect(isImplicit(connection({ implicit: true }))).toBe(true)
    expect(isImplicit(connection({}))).toBe(false)
  })
})

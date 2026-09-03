import { describe, expect, it, vi } from 'vitest'
import {
  isImplicitConnection,
  type InstalledConnectorPack,
  type SourceConnection
} from '../packages/shared/src/types'
import {
  syncImplicitConnection,
  type ImplicitConnectionDeps
} from '../packages/server/src/connectors/implicit-connection'

// The connection appears and disappears with the files; one made by hand is never withdrawn.

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
function deps(rows: SourceConnection[] = []) {
  return {
    list: (): SourceConnection[] => rows,
    create: vi.fn<ImplicitConnectionDeps['create']>((params) => connection(params.filters)),
    remove: vi.fn<ImplicitConnectionDeps['remove']>(),
    changed: vi.fn<ImplicitConnectionDeps['changed']>()
  } satisfies ImplicitConnectionDeps
}

describe('the connection a connector that needs no sign-in comes with', () => {
  it('is made when such a pack is installed, and handed back for discovery', () => {
    const d = deps()
    const made = syncImplicitConnection('echo-bench', NONE_PACK, d)
    expect(d.create).toHaveBeenCalledTimes(1)
    const params = d.create.mock.calls[0][0]
    expect(params.filters.implicit).toBe(true)
    expect(params.filters.sdkConnectorId).toBe('echo-bench')
    expect(params.name).toBe('Echo Bench')
    expect(made && isImplicitConnection(made)).toBe(true)
  })

  it('is not made a second time', () => {
    const d = deps([connection({ sdkConnectorId: 'echo-bench', implicit: true })])
    expect(syncImplicitConnection('echo-bench', NONE_PACK, d)).toBeUndefined()
    expect(d.create).not.toHaveBeenCalled()
    expect(d.remove).not.toHaveBeenCalled()
  })

  it('is withdrawn when the pack goes', () => {
    const d = deps([connection({ sdkConnectorId: 'echo-bench', implicit: true })])
    syncImplicitConnection('echo-bench', undefined, d)
    expect(d.remove).toHaveBeenCalledWith('existing-1')
    expect(d.changed).toHaveBeenCalled()
  })

  it('is withdrawn when an update gives the connector something to ask for', () => {
    const asksNow = { ...NONE_PACK, auth: { rung: 'key' as const, keys: ['token'] } }
    const d = deps([connection({ sdkConnectorId: 'echo-bench', implicit: true })])
    syncImplicitConnection('echo-bench', asksNow, d)
    expect(d.remove).toHaveBeenCalledWith('existing-1')
  })

  it('leaves a connection somebody made by hand alone', () => {
    const d = deps([connection({ sdkConnectorId: 'echo-bench' })])
    syncImplicitConnection('echo-bench', undefined, d)
    expect(d.remove).not.toHaveBeenCalled()
    expect(d.changed).not.toHaveBeenCalled()
  })

  it('is not made for a connector that asks for a key', () => {
    const d = deps()
    syncImplicitConnection(
      'echo-bench',
      { ...NONE_PACK, auth: { rung: 'key', keys: ['token'] } },
      d
    )
    expect(d.create).not.toHaveBeenCalled()
  })

  it('is not made for a connector that borrows a login', () => {
    const d = deps()
    syncImplicitConnection(
      'echo-bench',
      { ...NONE_PACK, auth: { rung: 'cli', probe: { command: 'glab', args: ['auth', 'status'] } } },
      d
    )
    expect(d.create).not.toHaveBeenCalled()
  })
})

describe('telling the app apart from a person', () => {
  it('knows which connection it made', () => {
    expect(isImplicitConnection(connection({ implicit: true }))).toBe(true)
    expect(isImplicitConnection(connection({}))).toBe(false)
  })
})

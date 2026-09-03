import { describe, expect, it, vi } from 'vitest'
import type { InstalledConnectorPack, SourceConnection } from '../packages/shared/src/types'
import {
  isImplicit,
  syncImplicitConnection,
  type ImplicitConnectionDeps
} from '../packages/server/src/connectors/implicit-connection'

/**
 * What happens to a connection the app made, when the person or the pack goes.
 *
 * The rule these pin is one-way: the app withdraws what it made, and leaves
 * what a person made alone. Refusing the delete is the other half — a row that
 * would come straight back on the next pack change is not the user's to remove.
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

function connection(filters: Record<string, unknown>, id = 'conn-1'): SourceConnection {
  return {
    id,
    connectorId: 'mcp',
    name: 'Echo Bench',
    filters,
    syncIntervalMinutes: 0,
    statusMapping: {},
    createdAt: new Date().toISOString()
  }
}

function deps(pack: InstalledConnectorPack | undefined, rows: SourceConnection[] = []) {
  return {
    describe: (): InstalledConnectorPack | undefined => pack,
    list: (): SourceConnection[] => rows,
    create: vi.fn<ImplicitConnectionDeps['create']>(),
    remove: vi.fn<ImplicitConnectionDeps['remove']>(),
    changed: vi.fn<ImplicitConnectionDeps['changed']>()
  } satisfies ImplicitConnectionDeps
}

/**
 * The count a removal reports, as `connector:removePack` builds it: every
 * connection of the connector that a person would have to make again.
 */
function connectionsThatStopWorking(rows: SourceConnection[]): number {
  return rows.filter((conn) => !isImplicit(conn)).length
}

describe('what a pack removal says it will cost', () => {
  it('does not count the connection that goes with the pack', () => {
    const rows = [
      connection({ sdkConnectorId: 'echo-bench', implicit: true }, 'implicit-1'),
      connection({ sdkConnectorId: 'echo-bench' }, 'by-hand-1')
    ]
    expect(connectionsThatStopWorking(rows)).toBe(1)
  })

  it('counts nothing when the only connection is the one the app made', () => {
    const rows = [connection({ sdkConnectorId: 'echo-bench', implicit: true })]
    expect(connectionsThatStopWorking(rows)).toBe(0)
  })
})

describe('reconciling a pack installed before the rule existed', () => {
  it('gives it the connection it should have had', () => {
    const d = deps(NONE_PACK, [])
    syncImplicitConnection('echo-bench', d)
    expect(d.create).toHaveBeenCalledTimes(1)
    expect(d.create.mock.calls[0][0].filters.implicit).toBe(true)
  })

  it('leaves a pack that already has its connection untouched', () => {
    const rows = [connection({ sdkConnectorId: 'echo-bench', implicit: true })]
    const d = deps(NONE_PACK, rows)
    syncImplicitConnection('echo-bench', d)
    expect(d.create).not.toHaveBeenCalled()
    expect(d.remove).not.toHaveBeenCalled()
  })
})

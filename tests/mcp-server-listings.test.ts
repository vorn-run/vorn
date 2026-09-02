import { describe, it, expect } from 'vitest'
import { buildConnectorListings } from '../src/renderer/lib/connector-browse'
import { canAddConnection } from '../src/renderer/lib/pack-status'
import type { McpServerCatalogEntry, SourceConnection } from '../packages/shared/src/types'

const SERVER: McpServerCatalogEntry = {
  id: 'playwright',
  name: 'Playwright',
  description: 'Drive a browser',
  command: 'npx',
  args: ['-y', '@playwright/mcp'],
  keywords: ['browser']
}

function connection(overrides: Partial<SourceConnection> = {}): SourceConnection {
  return {
    id: 'conn-1',
    name: 'Playwright',
    connectorId: 'mcp',
    filters: {},
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-01T00:00:00Z',
    ...overrides
  }
}

describe('MCP servers in the directory', () => {
  it('lists a published server beside the connectors', () => {
    const [listing] = buildConnectorListings([], [], [], [], [SERVER])
    expect(listing).toMatchObject({
      key: 'mcp:playwright',
      id: 'playwright',
      name: 'Playwright',
      source: 'mcp',
      category: 'MCP servers',
      keywords: ['browser'],
      connectedCount: 0
    })
    expect(listing.mcpServer).toEqual(SERVER)
  })

  it('keeps the published category when one is given', () => {
    const [listing] = buildConnectorListings([], [], [], [], [{ ...SERVER, category: 'Testing' }])
    expect(listing.category).toBe('Testing')
  })

  it('counts only the connections made to that server', () => {
    const listings = buildConnectorListings(
      [],
      [],
      [connection({ filters: { sdkConnectorId: 'playwright' } }), connection({ id: 'other' })],
      [],
      [SERVER]
    )
    const server = listings.find((l) => l.source === 'mcp')!
    expect(server.connectedCount).toBe(1)
  })

  it('can be connected to without installing anything first', () => {
    expect(canAddConnection({ kind: 'absent' }, { source: 'mcp' })).toBe(true)
  })

  it('lists a bare entry without inventing anything for it', () => {
    const bare = { id: 'tiny', name: 'Tiny', command: 'tiny-mcp', args: [] }
    const [listing] = buildConnectorListings([], [], [], [], [bare])
    expect(listing.keywords).toEqual([])
    expect(listing.category).toBe('MCP servers')
    expect(listing).not.toHaveProperty('description')
  })

  it('lists nothing when the catalog publishes no servers', () => {
    expect(buildConnectorListings([], [], [], [], [])).toEqual([])
  })
})

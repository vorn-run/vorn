import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { ExternalItem, SourceConnection } from '../packages/shared/src/types'

const { poll } = vi.hoisted(() => ({ poll: vi.fn() }))
vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartSdkClient: vi.fn(async () => ({ poll })),
  sessionGrantFor: () => undefined
}))

import { backfillSdkConnection } from '../packages/server/src/connectors/sdk'

function connection(filters: Record<string, unknown>): SourceConnection {
  return {
    id: 'c1',
    connectorId: 'sdk',
    name: 'Feeds',
    filters: { sdkConnectorId: 'rss', sdkVersion: '0.1.2', ...filters },
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-10T20:00:00Z'
  } as SourceConnection
}

const item = (id: string) => ({
  externalId: id,
  title: `Item ${id}`,
  url: `https://example.com/${id}`,
  description: `About ${id}`,
  status: 'new',
  labels: [],
  updatedAt: `2026-09-1${id}T00:00:00.000Z`
})

async function drain(conn: SourceConnection): Promise<ExternalItem[]> {
  const seen: ExternalItem[] = []
  await backfillSdkConnection(conn, (entry) => seen.push(entry))
  return seen
}

beforeEach(() => {
  poll.mockReset()
})

describe('backfillSdkConnection', () => {
  it('says a connection with no trigger has nothing to import', async () => {
    await expect(drain(connection({}))).rejects.toThrow(
      'Connection "Feeds" has no trigger, so there is nothing to import.'
    )
    expect(poll).not.toHaveBeenCalled()
  })

  it('follows the connector from no cursor while it says there is more', async () => {
    poll
      .mockResolvedValueOnce({ items: [item('1')], nextCursor: 'p2', hasMore: true })
      .mockResolvedValueOnce({ items: [item('2')], nextCursor: 'p3', hasMore: false })
    const seen = await drain(connection({ sdkTrigger: 'newItem' }))
    expect(seen.map((entry) => entry.externalId)).toEqual(['1', '2'])
    expect(poll.mock.calls.map(([params]) => params)).toEqual([
      { trigger: 'newItem' },
      { trigger: 'newItem', cursor: 'p2' }
    ])
  })

  it('stops a connector that says there is more without moving its cursor', async () => {
    poll.mockResolvedValue({ items: [item('1')], nextCursor: 'same', hasMore: true })
    await expect(drain(connection({ sdkTrigger: 'newItem' }))).rejects.toThrow(
      'reported more items without advancing its cursor'
    )
  })

  it('carries the fields backfill upserts on, and the whole item beside them', async () => {
    poll.mockResolvedValue({ items: [item('1')], hasMore: false })
    const [entry] = await drain(connection({ sdkTrigger: 'newItem' }))
    expect(entry).toEqual({
      externalId: '1',
      url: 'https://example.com/1',
      title: 'Item 1',
      description: 'About 1',
      status: 'new',
      updatedAt: '2026-09-11T00:00:00.000Z',
      metadata: item('1')
    })
  })
})

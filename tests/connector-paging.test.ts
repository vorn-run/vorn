import { describe, expect, it, vi } from 'vitest'
import type { ExternalItem, VornConnector } from '../packages/shared/src/types'
import { forEachConnectorItem } from '../packages/server/src/connectors/paging'

const item = (id: string): ExternalItem => ({
  externalId: id,
  url: `https://example.test/${id}`,
  title: id,
  description: '',
  status: 'open',
  updatedAt: '2026-08-04T00:00:00Z'
})

const connector = (overrides: Partial<VornConnector>): VornConnector => ({
  id: 'test',
  name: 'Test',
  icon: 'plug',
  capabilities: ['tasks'],
  describe: () => ({ auth: [] }),
  ...overrides
})

describe('forEachConnectorItem', () => {
  it('drains every bounded page in order', async () => {
    const listItemsPage = vi
      .fn()
      .mockResolvedValueOnce({ items: [item('1')], nextCursor: 'page-2', hasMore: true })
      .mockResolvedValueOnce({ items: [item('2')], hasMore: false })
    const seen: string[] = []

    await forEachConnectorItem(connector({ listItemsPage }), {}, (value) =>
      seen.push(value.externalId)
    )

    expect(seen).toEqual(['1', '2'])
    expect(listItemsPage).toHaveBeenNthCalledWith(1, {}, undefined)
    expect(listItemsPage).toHaveBeenNthCalledWith(2, {}, 'page-2')
  })

  it('supports a legacy one-shot listItems connector', async () => {
    const seen: string[] = []
    await forEachConnectorItem(
      connector({ listItems: vi.fn().mockResolvedValue([item('1')]) }),
      {},
      (value) => seen.push(value.externalId)
    )
    expect(seen).toEqual(['1'])
  })

  it('rejects a paged connector that cannot make progress', async () => {
    await expect(
      forEachConnectorItem(
        connector({
          listItemsPage: vi.fn().mockResolvedValue({
            items: [],
            nextCursor: 'same',
            hasMore: true
          })
        }),
        {},
        () => {}
      )
    ).rejects.toThrow(/did not advance/)
  })

  it('rejects a connector that cannot list items at all', async () => {
    await expect(forEachConnectorItem(connector({}), {}, () => {})).rejects.toThrow(
      /does not support listItems/
    )
  })

  it('stops an endlessly paging connector instead of looping forever', async () => {
    let page = 0
    await expect(
      forEachConnectorItem(
        connector({
          listItemsPage: vi.fn().mockImplementation(async () => ({
            items: [],
            nextCursor: `page-${++page}`,
            hasMore: true
          }))
        }),
        {},
        () => {}
      )
    ).rejects.toThrow(/exceeded 1000 backfill pages/)
  })
})

import { describe, it, expect, vi, beforeEach } from 'vitest'

const listConnectorCatalog = vi.fn()
const refreshRpc = vi.fn()
;(globalThis as unknown as { window: unknown }).window = {
  api: { listConnectorCatalog, refreshConnectorCatalog: refreshRpc }
}

const { refreshConnectorCatalog, __resetCatalogCacheForTests } =
  await import('../src/renderer/lib/use-connector-catalog')

const snapshot = (id: string) => ({
  items: [{ id, name: id, description: '', packageName: `p-${id}`, capabilities: [] }],
  templates: [],
  mcpServers: [],
  fetchedAt: 1
})

beforeEach(() => {
  __resetCatalogCacheForTests()
  vi.clearAllMocks()
  listConnectorCatalog.mockResolvedValue(snapshot('slack'))
  refreshRpc.mockResolvedValue(snapshot('gitlab'))
})

describe('checking the catalog again', () => {
  it('asks the publisher rather than re-reading the copy it already has', async () => {
    const next = await refreshConnectorCatalog()

    expect(refreshRpc).toHaveBeenCalled()
    expect(next.items[0].id).toBe('gitlab')
  })

  it('falls back to reading the catalog when this build cannot re-fetch', async () => {
    refreshRpc.mockResolvedValue(undefined)

    const next = await refreshConnectorCatalog()
    expect(listConnectorCatalog).toHaveBeenCalled()
    expect(next.items[0].id).toBe('slack')
  })

  it('caches an answer but never a silence', async () => {
    listConnectorCatalog.mockRejectedValueOnce(new Error('not connected'))
    refreshRpc.mockResolvedValue(undefined)

    expect((await refreshConnectorCatalog()).items).toEqual([])

    listConnectorCatalog.mockResolvedValue(snapshot('slack'))
    expect((await refreshConnectorCatalog()).items[0].id).toBe('slack')
  })
})

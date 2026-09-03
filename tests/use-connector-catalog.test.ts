import { describe, it, expect, vi, beforeEach, afterAll } from 'vitest'

const listConnectorCatalog = vi.fn()
const refreshRpc = vi.fn()
vi.stubGlobal('window', { api: { listConnectorCatalog, refreshConnectorCatalog: refreshRpc } })
afterAll(() => vi.unstubAllGlobals())

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

describe('reading the catalog', () => {
  it('keeps nothing when the build cannot ask yet, so the next panel asks again', async () => {
    refreshRpc.mockResolvedValue(undefined)
    listConnectorCatalog.mockResolvedValueOnce(undefined)
    expect((await refreshConnectorCatalog()).items).toEqual([])

    const next = await refreshConnectorCatalog()
    expect(listConnectorCatalog).toHaveBeenCalledTimes(2)
    expect(next.items[0].id).toBe('slack')
  })

  it('lets a refresh win over a read that started before it', async () => {
    let answerRead: (value: unknown) => void = () => {}
    listConnectorCatalog.mockReturnValueOnce(new Promise((resolve) => (answerRead = resolve)))
    refreshRpc.mockResolvedValue(undefined)
    const slow = refreshConnectorCatalog()
    refreshRpc.mockResolvedValue(snapshot('gitlab'))
    const fresh = await refreshConnectorCatalog()
    answerRead(snapshot('stale'))
    await slow
    expect(fresh.items[0].id).toBe('gitlab')
    expect((await refreshConnectorCatalog()).items[0].id).toBe('gitlab')
  })
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

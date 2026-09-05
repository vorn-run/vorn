// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, act, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ConnectorCatalogSnapshot } from '../src/shared/types'

const listConnectorCatalog = vi.fn()
const onConnectorCatalogChanged = vi.fn()
let announce: ((snapshot: ConnectorCatalogSnapshot) => void) | undefined
;(window as unknown as { api: unknown }).api = {
  listConnectorCatalog,
  onConnectorCatalogChanged
}

const { useConnectorCatalog, __resetCatalogCacheForTests } =
  await import('../src/renderer/lib/use-connector-catalog')

const snapshot = (id: string): ConnectorCatalogSnapshot => ({
  items: [
    {
      id,
      name: id,
      description: '',
      packageName: `@vornrun/connector-${id}`,
      capabilities: [],
      launch: { command: 'node', args: [] }
    }
  ],
  templates: [],
  mcpServers: [],
  fetchedAt: 1
})

function Probe(): React.ReactElement {
  const catalog = useConnectorCatalog()
  return <span data-testid="ids">{catalog.items.map((i) => i.id).join(',') || 'none'}</span>
}

beforeEach(() => {
  __resetCatalogCacheForTests()
  vi.clearAllMocks()
  listConnectorCatalog.mockResolvedValue(snapshot('slack'))
  onConnectorCatalogChanged.mockImplementation((cb: typeof announce) => {
    announce = cb
    return () => {}
  })
})

describe('a catalog the server refreshed on its own', () => {
  it('reaches a list already on screen, without anyone pressing refresh', async () => {
    render(<Probe />)
    await waitFor(() => expect(screen.getByTestId('ids')).toHaveTextContent('slack'))

    await act(async () => announce?.(snapshot('gitlab')))

    expect(screen.getByTestId('ids')).toHaveTextContent('gitlab')
    // The list came with the news, so nothing had to be asked for it again.
    expect(listConnectorCatalog).toHaveBeenCalledTimes(1)
  })

  it('subscribes once however many panels are open', async () => {
    render(<Probe />)
    render(<Probe />)
    await waitFor(() => expect(screen.getAllByTestId('ids')).toHaveLength(2))

    expect(onConnectorCatalogChanged).toHaveBeenCalledTimes(1)
  })
})

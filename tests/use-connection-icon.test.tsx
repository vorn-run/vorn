// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, waitFor, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import {
  refreshConnections,
  useConnectionIconFor,
  useConnectorGlyph,
  useConnectorIdFor,
  useConnectorLook,
  useInstalledPacks,
  __resetConnectionsCacheForTests
} from '../src/renderer/lib/use-connections'

const listConnections = vi.fn()
const listConnectorPacks = vi.fn()

beforeEach(() => {
  __resetConnectionsCacheForTests()
  listConnections.mockReset().mockResolvedValue([
    {
      id: 'packaged',
      connectorId: 'mcp',
      name: 'Azure Data Explorer',
      filters: { sdkIcon: JSON.stringify({ viewBox: '0 0 16 16', paths: ['M1 1h4v4z'] }) }
    },
    { id: 'plain', connectorId: 'github', name: 'GitHub', filters: {} },
    // Made before the connector shipped a glyph; the installed pack has one now.
    {
      id: 'iconless-pack',
      connectorId: 'mcp',
      name: 'Pack Demo',
      filters: { sdkConnectorId: 'packdemo' }
    }
  ])
  listConnectorPacks.mockReset().mockResolvedValue([
    {
      id: 'packdemo',
      name: 'Pack Demo',
      version: '1.0.0',
      icon: { viewBox: '0 0 24 24', paths: ['M2 2h9v9z'] }
    }
  ])
  ;(window as unknown as { api: unknown }).api = {
    listConnections,
    listConnectorPacks,
    onConfigChanged: () => () => {}
  }
})

function Probe({ connectionId }: { connectionId: string | null }) {
  const icon = useConnectionIconFor(connectionId)
  return <span data-testid="out">{icon ? icon.paths.join('|') : 'none'}</span>
}

describe('useConnectionIconFor', () => {
  it('resolves a packaged connector glyph from a connection id', async () => {
    const { getByTestId } = render(<Probe connectionId="packaged" />)

    await waitFor(() => expect(getByTestId('out')).toHaveTextContent('M1 1h4v4z'))
  })

  it('reports nothing for a connection that carries no glyph', async () => {
    const { getByTestId } = render(<Probe connectionId="plain" />)

    await waitFor(() => expect(listConnections).toHaveBeenCalled())
    expect(getByTestId('out')).toHaveTextContent('none')
  })

  it('reports nothing without a connection id, so nodes render before selection', () => {
    const { getByTestId } = render(<Probe connectionId={null} />)

    expect(getByTestId('out')).toHaveTextContent('none')
  })

  it('reports nothing for a connection that was deleted', async () => {
    const { getByTestId } = render(<Probe connectionId="gone" />)

    await waitFor(() => expect(listConnections).toHaveBeenCalled())
    expect(getByTestId('out')).toHaveTextContent('none')
  })

  it('falls back to the installed pack when the connection carries no glyph', async () => {
    const { getByTestId } = render(<Probe connectionId="iconless-pack" />)

    await waitFor(() => expect(getByTestId('out')).toHaveTextContent('M2 2h9v9z'))
  })

  it('survives a build whose preload cannot list packs', async () => {
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      listConnections,
      onConfigChanged: () => () => {}
    }
    const { getByTestId } = render(<Probe connectionId="packaged" />)

    await waitFor(() => expect(getByTestId('out')).toHaveTextContent('M1 1h4v4z'))
  })
})

function IdProbe({ connectionId }: { connectionId: string | null }) {
  return <span data-testid="out">{useConnectorIdFor(connectionId) ?? 'none'}</span>
}

function LookProbe({ connectionId }: { connectionId: string | null }) {
  const look = useConnectorLook(connectionId)
  const text = look ? `${look.connectorId}:${look.icon?.paths.join('|')}` : 'none'
  return <span data-testid="out">{text}</span>
}

function GlyphProbe({ connectorId }: { connectorId: string }) {
  const icon = useConnectorGlyph(connectorId)
  return <span data-testid="out">{icon ? icon.paths.join('|') : 'none'}</span>
}

describe('useConnectorIdFor', () => {
  it('names the connector a packaged connection really is, not the mcp it is stored as', async () => {
    const { getByTestId } = render(<IdProbe connectionId="iconless-pack" />)

    await waitFor(() => expect(getByTestId('out')).toHaveTextContent('packdemo'))
  })

  it('leaves a built-in connection alone', async () => {
    const { getByTestId } = render(<IdProbe connectionId="plain" />)

    await waitFor(() => expect(getByTestId('out')).toHaveTextContent('github'))
  })
})

describe('useConnectorLook', () => {
  it('answers with the real id and the glyph together', async () => {
    const { getByTestId } = render(<LookProbe connectionId="iconless-pack" />)

    await waitFor(() => expect(getByTestId('out')).toHaveTextContent('packdemo:M2 2h9v9z'))
  })

  it('reports nothing for a connection that was deleted', async () => {
    const { getByTestId } = render(<LookProbe connectionId="gone" />)

    await waitFor(() => expect(listConnections).toHaveBeenCalled())
    expect(getByTestId('out')).toHaveTextContent('none')
  })
})

describe('refreshConnections', () => {
  it('re-reads for a caller that just made a connection', async () => {
    const { getByTestId } = render(<Probe connectionId="new-one" />)
    await waitFor(() => expect(listConnections).toHaveBeenCalledTimes(1))
    expect(getByTestId('out')).toHaveTextContent('none')

    listConnections.mockResolvedValue([
      {
        id: 'new-one',
        connectorId: 'mcp',
        name: 'Fresh',
        filters: { sdkIcon: JSON.stringify({ viewBox: '0 0 16 16', paths: ['M9 9h1v1z'] }) }
      }
    ])
    await act(async () => {
      await refreshConnections()
    })

    expect(getByTestId('out')).toHaveTextContent('M9 9h1v1z')
  })

  it('survives a re-read the preload refuses', async () => {
    render(<Probe connectionId="packaged" />)
    await waitFor(() => expect(listConnections).toHaveBeenCalled())
    listConnections.mockRejectedValue(new Error('gone'))

    await expect(refreshConnections()).resolves.toBeUndefined()
  })
})

describe('useConnectorGlyph', () => {
  it('finds a glyph by connector id, for rows that never held a connection', async () => {
    const { getByTestId } = render(<GlyphProbe connectorId="packdemo" />)

    await waitFor(() => expect(getByTestId('out')).toHaveTextContent('M2 2h9v9z'))
  })

  it('reports nothing for a connector with no pack installed', async () => {
    const { getByTestId } = render(<GlyphProbe connectorId="github" />)

    await waitFor(() => expect(listConnections).toHaveBeenCalled())
    expect(getByTestId('out')).toHaveTextContent('none')
  })
})

function PacksProbe() {
  const packs = useInstalledPacks()
  return <span data-testid="out">{packs.map((pack) => pack.id).join(',') || 'none'}</span>
}

describe('useInstalledPacks', () => {
  it('reads the packs the connections refresh already fetched', async () => {
    const { getByTestId } = render(<PacksProbe />)

    await waitFor(() => expect(getByTestId('out')).toHaveTextContent('packdemo'))
    // One read served both, rather than a second round trip for the same answer.
    expect(listConnectorPacks).toHaveBeenCalledTimes(1)
  })

  it('sees a newly installed pack without being remounted', async () => {
    const { getByTestId } = render(<PacksProbe />)
    await waitFor(() => expect(getByTestId('out')).toHaveTextContent('packdemo'))

    listConnectorPacks.mockResolvedValue([
      { id: 'packdemo', name: 'Pack Demo', version: '1.0.0' },
      { id: 'slack', name: 'Slack', version: '1.2.0' }
    ])
    await act(async () => {
      await refreshConnections()
    })

    expect(getByTestId('out')).toHaveTextContent('packdemo,slack')
  })
})

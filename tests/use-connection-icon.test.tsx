// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import {
  useConnectionIconFor,
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
    { id: 'iconless-pack', connectorId: 'mcp', name: 'Pack Demo', filters: { sdkConnectorId: 'packdemo' } }
  ])
  listConnectorPacks
    .mockReset()
    .mockResolvedValue([
      { id: 'packdemo', name: 'Pack Demo', version: '1.0.0', icon: { viewBox: '0 0 24 24', paths: ['M2 2h9v9z'] } }
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

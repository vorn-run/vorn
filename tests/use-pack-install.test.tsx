// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { usePackInstall } from '../src/renderer/lib/use-pack-install'
import type { ConnectorListing } from '../src/renderer/lib/connector-browse'

const inspectConnectorPack = vi.fn()
const installConnectorPack = vi.fn()

const LISTING: ConnectorListing = {
  key: 'catalog:slack',
  id: 'slack',
  name: 'Slack',
  capabilities: ['actions'],
  category: 'Chat',
  source: 'catalog',
  keywords: [],
  connectedCount: 0,
  catalogItem: {
    id: 'slack',
    name: 'Slack',
    packageName: '@vornrun/connector-slack',
    packUrl: 'https://packs.test/slack-1.2.0.vorn.tgz',
    description: '',
    capabilities: ['actions'],
    launch: { command: 'node', args: [] }
  } as ConnectorListing['catalogItem']
}

beforeEach(() => {
  inspectConnectorPack.mockReset().mockResolvedValue({
    ok: true,
    preview: { id: 'slack', name: 'Slack', version: '1.2.0', token: 'tok-1' }
  })
  installConnectorPack.mockReset().mockResolvedValue({ ok: true, pack: { id: 'slack' } })
  ;(window as unknown as { api: unknown }).api = {
    inspectConnectorPack,
    installConnectorPack,
    onConnectorInstallProgress: () => () => {}
  }
})

function Probe({
  onInstalled,
  listing = LISTING
}: {
  onInstalled?: () => void
  listing?: ConnectorListing
}) {
  const install = usePackInstall(onInstalled)
  return (
    <div>
      <button onClick={() => void install.inspect(listing)}>inspect</button>
      <button onClick={() => void install.inspectFile('/tmp/pack.vorn.tgz')}>inspect file</button>
      <button onClick={() => void install.confirm()}>confirm</button>
      <button onClick={install.cancel}>cancel</button>
      <button onClick={install.clearError}>clear</button>
      <button onClick={() => install.report('two connections stopped')}>report</button>
      <span data-testid="pending">{install.pending?.preview.version ?? 'none'}</span>
      <span data-testid="error">{install.error ?? 'none'}</span>
      <span data-testid="failed">{install.progress.slack?.error ?? 'none'}</span>
      <span data-testid="phase">{install.progress.slack?.phase ?? 'none'}</span>
    </div>
  )
}

describe('installing a pack from wherever it was asked for', () => {
  it('asks before it keeps anything', async () => {
    render(<Probe />)
    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))
    expect(installConnectorPack).not.toHaveBeenCalled()
    // The published pack is preferred over the package it was built from.
    expect(inspectConnectorPack).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'url', url: 'https://packs.test/slack-1.2.0.vorn.tgz' })
    )
  })

  it('installs the files the sheet described, then says it is done', async () => {
    const onInstalled = vi.fn()
    render(<Probe onInstalled={onInstalled} />)
    fireEvent.click(screen.getByText('inspect'))
    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))

    fireEvent.click(screen.getByText('confirm'))

    await waitFor(() => expect(onInstalled).toHaveBeenCalled())
    expect(installConnectorPack).toHaveBeenCalledWith({ kind: 'staged', token: 'tok-1' })
    expect(screen.getByTestId('pending')).toHaveTextContent('none')
  })

  it('lands a refusal on the row that asked for it', async () => {
    inspectConnectorPack.mockResolvedValue({ ok: false, error: 'declares dependencies' })
    render(<Probe />)
    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() =>
      expect(screen.getByTestId('failed')).toHaveTextContent('declares dependencies')
    )
    expect(screen.getByTestId('pending')).toHaveTextContent('none')
  })

  it('falls back to the package a catalog entry was built from', async () => {
    const byName = { ...LISTING, catalogItem: { ...LISTING.catalogItem, packUrl: undefined } }
    render(<Probe listing={byName as ConnectorListing} />)
    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() =>
      expect(inspectConnectorPack).toHaveBeenCalledWith({
        kind: 'npm',
        packageName: '@vornrun/connector-slack'
      })
    )
  })

  it('follows an install the server is reporting on', async () => {
    let push: ((p: { id: string; phase: string; percent?: number }) => void) | undefined
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      inspectConnectorPack,
      installConnectorPack,
      onConnectorInstallProgress: (cb: typeof push) => {
        push = cb
        return () => {}
      }
    }
    render(<Probe />)

    act(() => push?.({ id: 'slack', phase: 'downloading', percent: 40 }))

    expect(screen.getByTestId('phase')).toHaveTextContent('downloading')
  })

  it('asks the same question of a pack already on this disk', async () => {
    render(<Probe />)
    fireEvent.click(screen.getByText('inspect file'))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))
    expect(inspectConnectorPack).toHaveBeenCalledWith({
      kind: 'file',
      path: '/tmp/pack.vorn.tgz'
    })
  })

  // A dropped file has no row to fail on, so its refusal is said outright.
  it('says why a file was refused, since no row can say it', async () => {
    inspectConnectorPack.mockResolvedValue({ ok: false, error: 'that is not a pack' })
    render(<Probe />)
    fireEvent.click(screen.getByText('inspect file'))

    await waitFor(() => expect(screen.getByTestId('error')).toHaveTextContent('that is not a pack'))
    expect(screen.getByTestId('pending')).toHaveTextContent('none')
  })

  it('drops the pending pack when the question is declined', async () => {
    render(<Probe />)
    fireEvent.click(screen.getByText('inspect'))
    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))

    fireEvent.click(screen.getByText('cancel'))

    expect(screen.getByTestId('pending')).toHaveTextContent('none')
    expect(installConnectorPack).not.toHaveBeenCalled()
  })

  it('says what a removal cost, and forgets it when asked', async () => {
    render(<Probe />)
    fireEvent.click(screen.getByText('report'))
    expect(screen.getByTestId('error')).toHaveTextContent('two connections stopped')

    fireEvent.click(screen.getByText('clear'))
    expect(screen.getByTestId('error')).toHaveTextContent('none')
  })

  // Nothing was kept, so there is nothing for the caller to re-read.
  it('does not announce an install that was refused', async () => {
    installConnectorPack.mockResolvedValue({ ok: false, error: 'declares dependencies' })
    const onInstalled = vi.fn()
    render(<Probe onInstalled={onInstalled} />)
    fireEvent.click(screen.getByText('inspect'))
    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))

    fireEvent.click(screen.getByText('confirm'))

    await waitFor(() => expect(screen.getByTestId('error')).toHaveTextContent('declares'))
    expect(onInstalled).not.toHaveBeenCalled()
  })

  // Both halves of the flow must speak to the row that started it, or a
  // refusal at the second step lands on a key no row is watching.
  it('reports a refused install on the row that asked, whatever it names itself', async () => {
    inspectConnectorPack.mockResolvedValue({
      ok: true,
      // The pack calls itself something else than the listing did.
      preview: { id: 'slack-connector', name: 'Slack', version: '1.2.0', token: 'tok-1' }
    })
    installConnectorPack.mockResolvedValue({ ok: false, error: 'declares dependencies' })
    render(<Probe />)
    fireEvent.click(screen.getByText('inspect'))
    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))

    fireEvent.click(screen.getByText('confirm'))

    await waitFor(() =>
      expect(screen.getByTestId('failed')).toHaveTextContent('declares dependencies')
    )
  })

  it('closes the sheet even when the install call itself fails', async () => {
    installConnectorPack.mockRejectedValue(new Error('the server went away'))
    render(<Probe />)
    fireEvent.click(screen.getByText('inspect'))
    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))

    fireEvent.click(screen.getByText('confirm'))

    await waitFor(() =>
      expect(screen.getByTestId('error')).toHaveTextContent('the server went away')
    )
    expect(screen.getByTestId('pending')).toHaveTextContent('none')
  })
})

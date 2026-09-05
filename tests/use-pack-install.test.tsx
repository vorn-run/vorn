// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { usePackInstall } from '../src/renderer/lib/use-pack-install'
import { matchesListing, type ConnectorListing } from '../src/renderer/lib/connector-browse'
import type { ConnectorPackSummary } from '../src/shared/types'

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

/** A row that already says everything the pack will: version, sign-in and receipt. */
const DESCRIBED: ConnectorListing = {
  ...LISTING,
  authRung: 'key',
  verified: {
    schema: 1,
    version: '1.2.0',
    checkedAt: '2026-09-04T00:00:00Z',
    checks: ['manifest']
  },
  catalogItem: { ...LISTING.catalogItem, version: '1.2.0' } as ConnectorListing['catalogItem']
}

const DESCRIBED_PREVIEW = {
  id: 'slack',
  name: 'Slack',
  version: '1.2.0',
  auth: { rung: 'key' as const, keys: ['botToken'] },
  token: 'tok-1'
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
  listing = LISTING,
  direct
}: {
  onInstalled?: () => void
  listing?: ConnectorListing
  /** What a surface that showed the pack's facts passes; the requirement row does not. */
  direct?: boolean
}) {
  const install = usePackInstall(onInstalled)
  return (
    <div>
      <button onClick={() => void install.inspect(listing, { ...(direct && { direct }) })}>
        inspect
      </button>
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

  it('asks for nothing when no release has published a pack', async () => {
    const unreleased: ConnectorListing = {
      ...LISTING,
      catalogItem: { ...LISTING.catalogItem, packUrl: undefined } as ConnectorListing['catalogItem']
    }
    render(<Probe listing={unreleased} />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('phase')).toHaveTextContent('none'))
    expect(inspectConnectorPack).not.toHaveBeenCalled()
    expect(screen.getByTestId('pending')).toHaveTextContent('none')
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

  // Pressing install on a row that already showed the version and the sign-in
  // is the answer; asking it again only adds a click.
  it('installs a pack the row already described, without asking again', async () => {
    inspectConnectorPack.mockResolvedValue({ ok: true, preview: DESCRIBED_PREVIEW })
    const onInstalled = vi.fn()
    render(<Probe onInstalled={onInstalled} listing={DESCRIBED} direct />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(onInstalled).toHaveBeenCalled())
    expect(installConnectorPack).toHaveBeenCalledTimes(1)
    expect(installConnectorPack).toHaveBeenCalledWith({ kind: 'staged', token: 'tok-1' })
    expect(screen.getByTestId('pending')).toHaveTextContent('none')
  })

  // A requirement row names the connector and nothing else, so it never asks to skip.
  it('asks from a surface that did not describe the pack, however well it matches', async () => {
    inspectConnectorPack.mockResolvedValue({ ok: true, preview: DESCRIBED_PREVIEW })
    render(<Probe listing={DESCRIBED} />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))
    expect(installConnectorPack).not.toHaveBeenCalled()
  })

  it('asks when the pack is a version the row did not advertise', async () => {
    inspectConnectorPack.mockResolvedValue({
      ok: true,
      preview: { ...DESCRIBED_PREVIEW, version: '1.3.0' }
    })
    render(<Probe listing={DESCRIBED} direct />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.3.0'))
    expect(installConnectorPack).not.toHaveBeenCalled()
  })

  it('asks when the pack signs in differently than the row said', async () => {
    inspectConnectorPack.mockResolvedValue({
      ok: true,
      preview: { ...DESCRIBED_PREVIEW, auth: { rung: 'cli' as const } }
    })
    render(<Probe listing={DESCRIBED} direct />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))
    expect(installConnectorPack).not.toHaveBeenCalled()
  })

  // Absent on either side means nobody stated it, and the row's badge is blank.
  it('asks when the pack states no sign-in', async () => {
    inspectConnectorPack.mockResolvedValue({
      ok: true,
      preview: { ...DESCRIBED_PREVIEW, auth: undefined }
    })
    render(<Probe listing={DESCRIBED} direct />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))
    expect(installConnectorPack).not.toHaveBeenCalled()
  })

  it('asks when the row states no sign-in', async () => {
    inspectConnectorPack.mockResolvedValue({ ok: true, preview: DESCRIBED_PREVIEW })
    render(<Probe listing={{ ...DESCRIBED, authRung: undefined }} direct />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))
    expect(installConnectorPack).not.toHaveBeenCalled()
  })

  it('asks when nothing vouched for the connector', async () => {
    inspectConnectorPack.mockResolvedValue({ ok: true, preview: DESCRIBED_PREVIEW })
    render(<Probe listing={{ ...DESCRIBED, verified: undefined }} direct />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))
    expect(installConnectorPack).not.toHaveBeenCalled()
  })

  // Replacing something already on disk is a decision, not a repeat of the row.
  it('asks when the pack would replace an installed version', async () => {
    inspectConnectorPack.mockResolvedValue({
      ok: true,
      preview: { ...DESCRIBED_PREVIEW, installedVersion: '1.1.0' }
    })
    render(<Probe listing={DESCRIBED} direct />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('pending')).toHaveTextContent('1.2.0'))
    expect(installConnectorPack).not.toHaveBeenCalled()
  })

  it('holds the row from the press, before the server has said anything', async () => {
    let answer: (value: unknown) => void = () => {}
    inspectConnectorPack.mockReturnValue(new Promise((resolve) => (answer = resolve)))
    render(<Probe />)

    fireEvent.click(screen.getByText('inspect'))

    expect(screen.getByTestId('phase')).toHaveTextContent('checking')
    await act(async () => answer({ ok: true, preview: DESCRIBED_PREVIEW }))
    // The sheet has the question now, so the row is free again.
    expect(screen.getByTestId('phase')).toHaveTextContent('none')
  })

  it('keeps the row installing until the reload has shown the pack', async () => {
    inspectConnectorPack.mockResolvedValue({ ok: true, preview: DESCRIBED_PREVIEW })
    let reloaded: () => void = () => {}
    const onInstalled = vi.fn(() => new Promise<void>((resolve) => (reloaded = resolve)))
    render(<Probe onInstalled={onInstalled} listing={DESCRIBED} direct />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(onInstalled).toHaveBeenCalled())
    expect(screen.getByTestId('phase')).toHaveTextContent('installing')
    await act(async () => reloaded())
    await waitFor(() => expect(screen.getByTestId('phase')).toHaveTextContent('none'))
  })

  it('frees the row and says so when the reload after an install fails', async () => {
    inspectConnectorPack.mockResolvedValue({ ok: true, preview: DESCRIBED_PREVIEW })
    const onInstalled = vi.fn(() => Promise.reject(new Error('server went away')))
    render(<Probe onInstalled={onInstalled} listing={DESCRIBED} direct />)

    fireEvent.click(screen.getByText('inspect'))

    await waitFor(() => expect(screen.getByTestId('phase')).toHaveTextContent('none'))
    expect(screen.getByTestId('error')).toHaveTextContent(
      'Installed, but the list could not be re-read'
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

describe('whether a checked pack is the one the row described', () => {
  const preview = DESCRIBED_PREVIEW as unknown as ConnectorPackSummary

  it('agrees when the id, the version, the sign-in and the receipt all do', () => {
    expect(matchesListing(preview, DESCRIBED)).toBe(true)
  })

  it('refuses to agree with a catalog that names no version', () => {
    expect(matchesListing(preview, LISTING)).toBe(false)
  })

  it('refuses to agree when the pack calls itself something else', () => {
    expect(matchesListing({ ...preview, id: 'slack-connector' }, DESCRIBED)).toBe(false)
    // A receipt for an older version vouches for that version, not this one.
    expect(
      matchesListing(preview, {
        ...DESCRIBED,
        verified: { ...DESCRIBED.verified!, version: '1.1.0' }
      })
    ).toBe(false)
  })

  it('refuses to read two unstated sign-ins as the same one', () => {
    const unstated = { ...preview, auth: undefined } as unknown as ConnectorPackSummary
    expect(matchesListing(unstated, { ...DESCRIBED, authRung: undefined })).toBe(false)
  })

  it('refuses to agree when nothing vouched for the connector', () => {
    expect(matchesListing(preview, { ...DESCRIBED, verified: undefined })).toBe(false)
  })
})

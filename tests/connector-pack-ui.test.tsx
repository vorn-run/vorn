// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { render, fireEvent, within } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type {
  ConnectorCatalogItem,
  InstalledConnectorPack,
  SourceConnection
} from '../packages/shared/src/types'
import { buildConnectorListings } from '../src/renderer/lib/connector-browse'
import {
  ConnectorDirectory,
  ConnectorRow
} from '../src/renderer/components/settings/ConnectorDirectory'
import { ConnectorDetail } from '../src/renderer/components/settings/ConnectorDetail'

const catalogEntry: ConnectorCatalogItem = {
  id: 'acme',
  name: 'Acme',
  description: 'Acme tickets',
  packageName: '@vornrun/connector-acme',
  version: '1.3.0',
  capabilities: ['triggers'],
  category: 'Development',
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-acme'] }
}

const installedPack = (
  overrides: Partial<InstalledConnectorPack> = {}
): InstalledConnectorPack => ({
  id: 'acme',
  name: 'Acme',
  version: '1.2.0',
  description: 'Acme tickets',
  path: '/data/connectors/acme/1.2.0',
  installedAt: 0,
  bytes: 2048,
  triggers: [
    {
      type: 'newTicket',
      label: 'New ticket',
      description: 'Tickets opened since the last poll',
      filters: {
        pollTool: 'poll_newTicket',
        itemsPath: 'items',
        idField: 'externalId',
        timestampField: 'updatedAt',
        titleField: 'title',
        urlField: 'url',
        cursorArg: 'cursor',
        cursorPath: 'nextCursor'
      }
    }
  ],
  actions: [{ type: 'closeTicket', label: 'Close ticket' }],
  env: [{ name: 'API_TOKEN', required: true, secret: true }],
  ...overrides
})

function listingFor(packs: InstalledConnectorPack[], catalog = [catalogEntry]) {
  const listings = buildConnectorListings([], catalog, [] as SourceConnection[], packs)
  return listings[0]
}

describe('buildConnectorListings installed arm', () => {
  it('attaches the pack to the catalog row rather than adding a second one', () => {
    const listings = buildConnectorListings([], [catalogEntry], [], [installedPack()])
    expect(listings).toHaveLength(1)
    expect(listings[0].source).toBe('catalog')
    expect(listings[0].pack?.version).toBe('1.2.0')
  })

  it('lists a pack the catalog does not carry, describing it from its manifest', () => {
    const listings = buildConnectorListings([], [], [], [installedPack({ id: 'sideloaded' })])
    expect(listings).toHaveLength(1)
    expect(listings[0]).toMatchObject({
      key: 'installed:sideloaded',
      source: 'installed',
      name: 'Acme',
      category: 'Installed',
      capabilities: ['triggers', 'actions']
    })
  })

  it('leaves the catalog alone when nothing is installed', () => {
    const listings = buildConnectorListings([], [catalogEntry], [])
    expect(listings[0].pack).toBeUndefined()
  })
})

describe('ConnectorRow pack states', () => {
  const setup = (
    listing: ReturnType<typeof listingFor>,
    props: Partial<Parameters<typeof ConnectorRow>[0]> = {}
  ) =>
    render(
      <ConnectorRow
        listing={listing}
        builtIns={[]}
        onSelect={() => {}}
        onAdd={() => {}}
        onInstall={() => {}}
        {...props}
      />
    )

  it('offers Install and withholds Add for a connector with no package route', () => {
    const listing = { ...listingFor([]), catalogItem: undefined, source: 'installed' as const }
    const { container } = setup(listing)
    expect(within(container).getByText('Install')).toBeInTheDocument()
    expect(within(container).queryByText('Add')).not.toBeInTheDocument()
  })

  it('keeps Add on a catalog entry that can still launch from its package name', () => {
    const { getByText } = setup(listingFor([]))
    expect(getByText('Add')).toBeInTheDocument()
    expect(getByText('Install')).toBeInTheDocument()
  })

  it('draws a progress hairline while downloading and disables the button', () => {
    const { getByRole, getByText } = setup(listingFor([]), {
      progress: { id: 'acme', phase: 'downloading', percent: 40 }
    })
    const bar = getByRole('progressbar')
    expect(bar).toHaveAttribute('aria-valuenow', '40')
    expect(bar).toHaveStyle({ width: '40%' })
    expect(getByText('Downloading')).toBeInTheDocument()
  })

  it('says it is verifying, with no bar, once the bytes are in', () => {
    const { queryByRole, getByText } = setup(listingFor([]), {
      progress: { id: 'acme', phase: 'verifying' }
    })
    expect(queryByRole('progressbar')).not.toBeInTheDocument()
    expect(getByText('Verifying')).toBeInTheDocument()
  })

  it('shows the installed version and drops the install action once up to date', () => {
    const { container } = setup(listingFor([installedPack({ version: '1.3.0' })]))
    expect(within(container).getByText('v1.3.0')).toBeInTheDocument()
    expect(within(container).queryByText('Install')).not.toBeInTheDocument()
    expect(within(container).getByText('Add')).toBeInTheDocument()
  })

  it('offers the update when the catalog publishes a newer version', () => {
    const { getByText } = setup(listingFor([installedPack()]))
    expect(getByText('v1.2.0 → 1.3.0 available')).toBeInTheDocument()
    expect(getByText('Update')).toBeInTheDocument()
  })

  it('shows a rejection in danger and lets it be retried', () => {
    const onInstall = vi.fn()
    const { getByText } = setup(listingFor([]), {
      progress: { id: 'acme', phase: 'failed', error: 'declares dependencies' },
      onInstall
    })
    const line = getByText('declares dependencies')
    expect(line.className).toContain('text-danger')
    fireEvent.click(getByText("Couldn't install"))
    expect(onInstall).toHaveBeenCalled()
  })

  it('offers no install control for a built-in', () => {
    const builtIn = buildConnectorListings(
      [{ id: 'github', name: 'GitHub', capabilities: ['tasks'] }],
      [],
      []
    )[0]
    const { container } = setup(builtIn)
    expect(within(container).queryByText('Install')).not.toBeInTheDocument()
    expect(within(container).getByText('Add')).toBeInTheDocument()
  })
})

describe('ConnectorDetail pack footer', () => {
  const setup = (
    listing: ReturnType<typeof listingFor>,
    props: Partial<Parameters<typeof ConnectorDetail>[0]> = {}
  ) =>
    render(
      <ConnectorDetail
        listing={listing}
        builtIns={[]}
        onAdd={() => {}}
        onClose={() => {}}
        {...props}
      />
    )

  it('says what is on disk and how it is launched', () => {
    const { getByText } = setup(listingFor([installedPack()]))
    expect(getByText('On this machine')).toBeInTheDocument()
    expect(getByText('/data/connectors/acme/1.2.0')).toBeInTheDocument()
    expect(getByText('node /data/connectors/acme/1.2.0/index.js')).toBeInTheDocument()
  })

  it('describes an installed pack from its own manifest, not the catalog', () => {
    const { getByText } = setup(listingFor([installedPack()], []))
    expect(getByText('New ticket')).toBeInTheDocument()
    expect(getByText('Close ticket')).toBeInTheDocument()
    expect(getByText(/API_TOKEN/)).toBeInTheDocument()
  })

  it('withholds a rollback while there is no version behind the current one', () => {
    const { container } = setup(listingFor([installedPack()]), { onRollback: () => {} })
    expect(within(container).queryByText('Roll back')).not.toBeInTheDocument()
  })

  it('offers a rollback once a version is kept behind the current one', () => {
    const { container } = setup(listingFor([installedPack({ previousVersion: '1.1.0' })]), {
      onRollback: () => {}
    })
    expect(within(container).getByText('Roll back')).toBeInTheDocument()
  })

  it('offers Remove for something actually on disk', () => {
    const { container } = setup(listingFor([installedPack()]), { onRemove: () => {} })
    expect(within(container).getByText('Remove')).toBeInTheDocument()
  })

  it('withholds Remove when there is nothing installed to delete', () => {
    const { container } = setup(listingFor([]), { onRemove: () => {} })
    expect(within(container).queryByText('Remove')).not.toBeInTheDocument()
  })

  it('runs the footer actions it is handed', () => {
    const onInstall = vi.fn()
    const onRollback = vi.fn()
    const onRemove = vi.fn()
    const { getByText } = setup(listingFor([installedPack({ previousVersion: '1.1.0' })]), {
      onInstall,
      onRollback,
      onRemove
    })

    fireEvent.click(getByText('Update'))
    fireEvent.click(getByText('Roll back'))
    fireEvent.click(getByText('Remove'))
    expect(onInstall).toHaveBeenCalled()
    expect(onRollback).toHaveBeenCalled()
    expect(onRemove).toHaveBeenCalled()
  })
})

describe('installing a pack from a file', () => {
  const setup = (props: Partial<Parameters<typeof ConnectorDirectory>[0]> = {}) =>
    render(
      <ConnectorDirectory
        listings={buildConnectorListings([], [catalogEntry], [])}
        builtIns={[]}
        onSelect={() => {}}
        onAdd={() => {}}
        {...props}
      />
    )

  const dropFiles = (target: Element, files: Array<{ path?: string }>): void => {
    fireEvent.drop(target, { dataTransfer: { files } })
  }

  it('installs each dropped file by the path it came with', () => {
    const onInstallFile = vi.fn()
    const { container } = setup({ onInstallFile })
    dropFiles(container.firstElementChild as Element, [
      { path: '/tmp/acme-1.2.0.vorn.tgz' },
      { path: '/tmp/other-2.0.0.vorn.tgz' }
    ])
    expect(onInstallFile).toHaveBeenCalledTimes(2)
    expect(onInstallFile).toHaveBeenCalledWith('/tmp/acme-1.2.0.vorn.tgz')
  })

  it('ignores a drop carrying nothing with a path on disk', () => {
    const onInstallFile = vi.fn()
    const { container } = setup({ onInstallFile })
    dropFiles(container.firstElementChild as Element, [{}])
    expect(onInstallFile).not.toHaveBeenCalled()
  })

  it('marks the drop target while a file is over it, and clears it after', () => {
    const { container } = setup({ onInstallFile: () => {} })
    const target = container.firstElementChild as Element
    fireEvent.dragOver(target)
    expect(target).toHaveAttribute('data-drop-active', 'true')
    fireEvent.drop(target, { dataTransfer: { files: [] } })
    expect(target).not.toHaveAttribute('data-drop-active')
  })

  it('accepts no drop at all when installing from a file is not offered', () => {
    const { container } = setup()
    const target = container.firstElementChild as Element
    fireEvent.dragOver(target)
    expect(target).not.toHaveAttribute('data-drop-active')
  })

  it('installs what the file picker returns and skips a cancelled pick', async () => {
    const onInstallFile = vi.fn()
    const picked = setup({ onInstallFile, onPickFile: async () => '/tmp/acme-1.2.0.vorn.tgz' })
    fireEvent.click(within(picked.container).getByText('Install from file'))
    await vi.waitFor(() => expect(onInstallFile).toHaveBeenCalledWith('/tmp/acme-1.2.0.vorn.tgz'))

    const cancelled = setup({ onInstallFile, onPickFile: async () => null })
    fireEvent.click(within(cancelled.container).getByText('Install from file'))
    await vi.waitFor(() => expect(onInstallFile).toHaveBeenCalledTimes(1))
  })

  it('hides the file button when there is no picker to open', () => {
    const { container } = setup({ onInstallFile: () => {} })
    expect(within(container).queryByText('Install from file')).not.toBeInTheDocument()
  })

  it('reports a refused file install above the list, where it has no row', () => {
    const { container } = setup({
      onInstallFile: () => {},
      installError: 'The pack has no manifest.json'
    })
    const line = within(container).getByText('The pack has no manifest.json')
    expect(line.className).toContain('text-danger')
  })
})

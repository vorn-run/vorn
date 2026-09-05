// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ConnectorDirectory } from '../src/renderer/components/settings/ConnectorDirectory'
import { ConnectorDetail } from '../src/renderer/components/settings/ConnectorDetail'
import {
  buildConnectorListings,
  type BuiltInConnector,
  type ConnectorListing
} from '../src/renderer/lib/connector-browse'
import type { ConnectorCatalogItem } from '../src/shared/types'

const ADO: ConnectorCatalogItem = {
  id: 'ado',
  name: 'Azure DevOps',
  description: 'Trigger workflows from the work items a WIQL query returns.',
  packageName: '@vornrun/connector-ado',
  version: '0.1.0',
  packUrl: 'https://packs.test/ado-0.1.0.vorn.tgz',
  capabilities: ['triggers'],
  category: 'Development',
  keywords: ['boards', 'tfs'],
  auth: 'Signs in with your Azure identity.',
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-ado'] },
  triggers: [
    {
      type: 'workItem',
      label: 'Work item matches the query',
      description: 'Fires once per work item the WIQL query newly returns.'
    }
  ],
  actions: [],
  env: [
    { name: 'ADO_ORGANIZATION', required: true },
    { name: 'ADO_TOP', required: false }
  ]
}

const KUSTO: ConnectorCatalogItem = {
  id: 'kusto',
  name: 'Azure Data Explorer',
  description: 'Trigger workflows from the rows a KQL query returns.',
  packageName: '@vornrun/connector-kusto',
  version: '0.6.0',
  packUrl: 'https://packs.test/kusto-0.6.0.vorn.tgz',
  capabilities: ['triggers', 'actions'],
  category: 'Data & observability',
  keywords: ['kql'],
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-kusto'] },
  triggers: [{ type: 'queryResult', label: 'Query returns a row' }],
  actions: [{ type: 'runQuery', label: 'Run a KQL query' }],
  env: [{ name: 'KUSTO_CLUSTER', required: true }]
}

/** A catalog entry no release has published a pack for yet. */
const UNRELEASED: ConnectorCatalogItem = {
  ...KUSTO,
  id: 'gitlab',
  name: 'GitLab',
  packageName: '@vornrun/connector-gitlab',
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-gitlab'] },
  packUrl: undefined
}

const listings = (): ConnectorListing[] => buildConnectorListings([], [ADO, KUSTO], [])

const find = (id: string) => listings().find((listing) => listing.id === id)!

describe('the connector list', () => {
  const setup = () => {
    const onSelect = vi.fn()
    const onAdd = vi.fn()
    const utils = render(
      <ConnectorDirectory listings={listings()} builtIns={[]} onSelect={onSelect} onAdd={onAdd} />
    )
    return { ...utils, onSelect, onAdd }
  }

  it('says a connector no release has published yet is not installable', () => {
    const { getByText, queryByText } = render(
      <ConnectorDirectory
        listings={buildConnectorListings([], [UNRELEASED], [])}
        builtIns={[]}
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        onInstall={vi.fn()}
      />
    )

    expect(getByText('Not released yet')).toBeInTheDocument()
    expect(queryByText('Install')).not.toBeInTheDocument()
  })

  it('calls connecting to an MCP server adding a server, and offers no install', () => {
    const server = { id: 'playwright', name: 'Playwright', command: 'npx', args: [] }
    const onAdd = vi.fn()
    const { getByText, queryByText } = render(
      <ConnectorDirectory
        listings={buildConnectorListings([], [], [], [], [server])}
        builtIns={[]}
        onSelect={vi.fn()}
        onAdd={onAdd}
        onInstall={vi.fn()}
      />
    )

    // There is no pack behind a server, so an Install button would be a lie.
    expect(queryByText('Install')).not.toBeInTheDocument()
    fireEvent.click(getByText('Add server'))
    expect(onAdd).toHaveBeenCalled()
  })

  it('says what a connector offers, and what would be installed, in one line', () => {
    // A name and a blurb cannot answer "will this do what I need", and both
    // blurbs here start with the same four words.
    const { getByText } = setup()
    expect(getByText('Development · 1 trigger · v0.1.0')).toBeInTheDocument()
    expect(getByText('Data & observability · 1 trigger, 1 action · v0.6.0')).toBeInTheDocument()
  })

  it('offers to add another of something already in use', () => {
    const inUse = buildConnectorListings(
      [],
      [ADO, KUSTO],
      [{ id: 'c1', connectorId: 'ado', name: 'board', filters: {} } as never]
    )
    const { getByText } = render(
      <ConnectorDirectory listings={inUse} builtIns={[]} onSelect={vi.fn()} onAdd={vi.fn()} />
    )
    expect(getByText('Add another')).toBeInTheDocument()
    expect(getByText(/Development · 1 trigger · v0\.1\.0 · in use/)).toBeInTheDocument()
  })

  it('adds without making anyone open the details first', () => {
    const { getAllByText, onAdd } = setup()
    fireEvent.click(getAllByText('Add')[0])
    expect(onAdd).toHaveBeenCalled()
  })

  it('finds a connector by a word that is nowhere in its name', () => {
    const { getByPlaceholderText, queryByText, getByText } = setup()
    fireEvent.change(getByPlaceholderText('Search connectors'), { target: { value: 'tfs' } })

    expect(getByText('Azure DevOps')).toBeInTheDocument()
    expect(queryByText('Azure Data Explorer')).not.toBeInTheDocument()
  })

  it('narrows to one category', () => {
    const { getByLabelText, getByText, queryByText } = setup()
    fireEvent.change(getByLabelText('Filter by category'), { target: { value: 'Development' } })

    expect(getByText('Azure DevOps')).toBeInTheDocument()
    expect(queryByText('Azure Data Explorer')).not.toBeInTheDocument()
  })

  it('narrows to what a workflow step can call', () => {
    const { getByLabelText, getByText, queryByText } = setup()
    fireEvent.change(getByLabelText('Filter by category'), {
      target: { value: 'Can be called from a step' }
    })

    expect(getByText('Azure Data Explorer')).toBeInTheDocument()
    expect(queryByText('Azure DevOps')).not.toBeInTheDocument()
  })

  it('goes back to everything, so a filter is never a trap', () => {
    const { getByLabelText, getByText } = setup()
    fireEvent.change(getByLabelText('Filter by category'), { target: { value: 'Development' } })
    fireEvent.change(getByLabelText('Filter by category'), { target: { value: '' } })
    expect(getByText('Azure Data Explorer')).toBeInTheDocument()
  })

  it('says nothing matched rather than looking empty and broken', () => {
    const { getByPlaceholderText, getByText } = setup()
    fireEvent.change(getByPlaceholderText('Search connectors'), { target: { value: 'zzz' } })
    expect(getByText('No connectors match that.')).toBeInTheDocument()
  })

  it('says how current the list is, and offers to check again', async () => {
    const onRefresh = vi.fn()
    const { getByText } = render(
      <ConnectorDirectory
        listings={listings()}
        builtIns={[]}
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        fetchedAt={Date.now() - 2 * 3_600_000}
        onRefresh={onRefresh}
      />
    )
    expect(getByText('Updated 2 hours ago')).toBeInTheDocument()
    fireEvent.click(getByText('Check now'))
    expect(onRefresh).toHaveBeenCalled()
  })

  it('leaves the freshness line out when there is nothing to refresh', () => {
    const { queryByText } = setup()
    expect(queryByText('Check now')).not.toBeInTheDocument()
  })

  it('reports the row that was opened', () => {
    const { getByText, onSelect } = setup()
    fireEvent.click(getByText('Azure DevOps'))
    expect(onSelect).toHaveBeenCalledWith(expect.objectContaining({ id: 'ado' }))
  })

  it('says a row opens something, rather than leaving Add as the only clue', () => {
    const { getAllByText, onSelect } = setup()
    fireEvent.click(getAllByText('Details')[0])
    expect(onSelect).toHaveBeenCalled()
  })
})

describe('where a verified pack asks to be kept', () => {
  const sheet = <div>Keep this pack?</div>

  const order = (container: HTMLElement, text: string) => container.textContent!.indexOf(text)

  it('asks under the row the install began on', () => {
    const { container } = render(
      <ConnectorDirectory
        listings={listings()}
        builtIns={[]}
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        pending={{ sheet, rowKey: find('kusto').key }}
      />
    )

    // Between its own row and the next, rather than appended after the list.
    expect(order(container, 'Keep this pack?')).toBeGreaterThan(
      order(container, 'Azure Data Explorer')
    )
    expect(order(container, 'Keep this pack?')).toBeLessThan(order(container, 'Azure DevOps'))
  })

  it('asks above the list when no row owns the pack, as a dropped file has none', () => {
    const { container } = render(
      <ConnectorDirectory
        listings={listings()}
        builtIns={[]}
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        pending={{ sheet }}
      />
    )

    expect(order(container, 'Keep this pack?')).toBeLessThan(order(container, 'Azure DevOps'))
  })

  it('asks under the buttons on the page that raised it', () => {
    const { container, getByText } = render(
      <ConnectorDetail
        listing={find('ado')}
        builtIns={[]}
        onAdd={vi.fn()}
        onClose={vi.fn()}
        onInstall={vi.fn()}
        pending={sheet}
      />
    )

    expect(getByText('Install')).toBeInTheDocument()
    expect(order(container, 'Keep this pack?')).toBeGreaterThan(order(container, 'Install'))
  })
})

describe('a connector page for something not released', () => {
  it('offers no install, and says why', () => {
    const listing = buildConnectorListings([], [UNRELEASED], [])[0]
    const { getByText, queryByText } = render(
      <ConnectorDetail
        listing={listing}
        builtIns={[]}
        onAdd={vi.fn()}
        onClose={vi.fn()}
        onInstall={vi.fn()}
      />
    )

    expect(getByText(/Not released yet/)).toBeInTheDocument()
    expect(queryByText('Install')).not.toBeInTheDocument()
  })
})

describe('the connector detail panel', () => {
  const setup = (listing: ConnectorListing, builtIns: BuiltInConnector[] = []) => {
    const onAdd = vi.fn()
    const onClose = vi.fn()
    const utils = render(
      <ConnectorDetail listing={listing} builtIns={builtIns} onAdd={onAdd} onClose={onClose} />
    )
    return { ...utils, onAdd, onClose }
  }

  it('shows each trigger verbatim, description and all', () => {
    const { getByText } = setup(find('ado'))
    expect(getByText('Work item matches the query')).toBeInTheDocument()
    expect(getByText(/Fires once per work item the WIQL query newly returns/)).toBeInTheDocument()
  })

  it('says a watcher only watches instead of showing an empty heading', () => {
    const { getByText } = setup(find('ado'))
    expect(getByText('Nothing — this one only watches.')).toBeInTheDocument()
  })

  it('lists the actions a workflow step could call', () => {
    const { getByText } = setup(find('kusto'))
    expect(getByText('Run a KQL query')).toBeInTheDocument()
  })

  it('shows the settings it will ask for, before the form does', () => {
    // Discovering a required setting on step three of a form is the thing this
    // panel exists to prevent.
    const { getByText } = setup(find('ado'))
    expect(getByText('ADO_ORGANIZATION')).toBeInTheDocument()
    // Optional says so in words rather than by a border style nobody decodes.
    expect(getByText('ADO_TOP (optional)')).toBeInTheDocument()
  })

  it('names the package and version, so what would be installed is not a mystery', () => {
    const { getByText } = setup(find('ado'))
    expect(getByText(/@vornrun\/connector-ado · v0\.1\.0/)).toBeInTheDocument()
  })

  it('offers a way back to the list it replaced', () => {
    const { getByText, onClose } = setup(find('ado'))
    fireEvent.click(getByText('All connectors'))
    expect(onClose).toHaveBeenCalled()
  })

  it('says it knows nothing rather than claiming a connector does nothing', () => {
    // An entry from a catalog published before triggers were carried.
    const older = buildConnectorListings(
      [],
      [{ ...ADO, triggers: undefined, actions: undefined }],
      []
    )[0]
    const { getByText, queryByText } = setup(older)

    expect(getByText(/does not describe itself yet/)).toBeInTheDocument()
    expect(queryByText('Nothing — this one only watches.')).not.toBeInTheDocument()
  })

  it('describes a built-in from the manifest already in the process', () => {
    const github: BuiltInConnector = {
      id: 'github',
      name: 'GitHub',
      capabilities: ['triggers'],
      manifest: { triggers: [{ type: 'issue', label: 'An issue is opened' }] }
    }
    const listing = buildConnectorListings([github], [], [])[0]
    const { getByText } = setup(listing, [github])
    expect(getByText('An issue is opened')).toBeInTheDocument()
  })

  it('hands Add back rather than installing anything itself', () => {
    const { getByText, onAdd } = setup(find('ado'))
    fireEvent.click(getByText('Add a connection'))
    expect(onAdd).toHaveBeenCalled()
  })

  it('is honest that reading about a connector installs nothing', () => {
    const { getByText } = setup(find('ado'))
    expect(getByText(/Nothing is on disk until you install it/)).toBeInTheDocument()
  })
})

describe('a list too long to scan flat', () => {
  const SLACK: ConnectorCatalogItem = {
    ...KUSTO,
    id: 'slack',
    name: 'Slack',
    description: 'Messages and channels.',
    packageName: '@vornrun/connector-slack',
    category: 'Chat',
    authRung: 'key',
    verified: {
      schema: 1,
      version: '0.6.0',
      checkedAt: '2026-09-02T00:00:00Z',
      checks: ['manifest', 'no-runtime-deps']
    },
    launch: { command: 'npx', args: ['-y', '@vornrun/connector-slack'] }
  }
  const GITLAB: ConnectorCatalogItem = {
    ...ADO,
    id: 'gitlab',
    name: 'GitLab',
    packageName: '@vornrun/connector-gitlab',
    category: 'Development',
    authRung: 'cli',
    launch: { command: 'npx', args: ['-y', '@vornrun/connector-gitlab'] }
  }

  const draw = () =>
    render(
      <ConnectorDirectory
        listings={buildConnectorListings([], [ADO, SLACK, GITLAB], [])}
        builtIns={[]}
        onSelect={vi.fn()}
        onAdd={vi.fn()}
      />
    )

  it('heads each category once, in the order the list already sorted them', () => {
    const { container } = draw()
    const headings = [...container.querySelectorAll('h3')].map((h) => h.textContent)
    expect(headings).toEqual(['Development', 'Chat'])
  })

  it('says on the row how a connector signs in, and that it was checked', () => {
    const { getByText, getAllByText } = draw()
    expect(getByText('CLI login')).toBeInTheDocument()
    expect(getByText('key')).toBeInTheDocument()
    expect(getAllByText('verified')).toHaveLength(1)
  })

  it('narrows to the connectors that ask the same thing of you', () => {
    const { getByLabelText, queryByText, getByText } = draw()
    fireEvent.change(getByLabelText('Filter by sign-in'), { target: { value: 'cli' } })

    expect(getByText('GitLab')).toBeInTheDocument()
    expect(queryByText('Slack')).not.toBeInTheDocument()
    expect(queryByText('Azure DevOps')).not.toBeInTheDocument()
  })

  it('shows the receipt behind the badge, not just the word', () => {
    const listing = buildConnectorListings([], [SLACK], []).find((l) => l.id === 'slack')!
    const { getByText } = render(
      <ConnectorDetail listing={listing} builtIns={[]} onAdd={vi.fn()} onClose={vi.fn()} />
    )

    expect(getByText('manifest')).toBeInTheDocument()
    expect(getByText('no-runtime-deps')).toBeInTheDocument()
    expect(getByText(/checked .* against v0\.6\.0/)).toBeInTheDocument()
    expect(getByText('Needs a key')).toBeInTheDocument()
  })

  it('offers no sign-in filter when every connector answers it the same way', () => {
    const { queryByLabelText } = render(
      <ConnectorDirectory
        listings={buildConnectorListings([], [ADO, KUSTO], [])}
        builtIns={[]}
        onSelect={vi.fn()}
        onAdd={vi.fn()}
      />
    )
    expect(queryByLabelText('Filter by sign-in')).not.toBeInTheDocument()
  })
})

describe('a connector that signs in with nothing', () => {
  const ECHO = {
    id: 'echo',
    name: 'Echo Bench',
    version: '1.0.0',
    path: '/packs/echo',
    installedAt: 0,
    bytes: 1,
    auth: { rung: 'none' as const },
    triggers: [],
    actions: [{ type: 'echo', label: 'Echo' }],
    env: []
  }
  const implicitConnection = {
    id: 'implicit-1',
    connectorId: 'mcp',
    name: 'Echo Bench',
    filters: { sdkConnectorId: 'echo', implicit: true },
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-02T00:00:00Z'
  }

  it('offers the workflow rather than a connection nobody needs', () => {
    const [listing] = buildConnectorListings([], [], [implicitConnection as never], [ECHO])
    const onUse = vi.fn()
    const { getByText, queryByText } = render(
      <ConnectorDetail
        listing={listing}
        builtIns={[]}
        onAdd={vi.fn()}
        onUse={onUse}
        onClose={vi.fn()}
      />
    )

    expect(queryByText('Add a connection')).not.toBeInTheDocument()
    expect(getByText(/Nothing — ready as soon as it is installed/)).toBeInTheDocument()
    fireEvent.click(getByText('Use in a workflow'))
    expect(onUse).toHaveBeenCalled()
  })

  it('says so plainly rather than offering a button that goes nowhere', () => {
    const [listing] = buildConnectorListings([], [], [implicitConnection as never], [ECHO])
    const { getByText, queryByText } = render(
      <ConnectorDetail listing={listing} builtIns={[]} onAdd={vi.fn()} onClose={vi.fn()} />
    )

    expect(queryByText('Use in a workflow')).not.toBeInTheDocument()
    expect(getByText('Ready to use in a workflow.')).toBeInTheDocument()
  })

  it('asks nobody to add a connection it already made', () => {
    const { queryByText, getByText } = render(
      <ConnectorDirectory
        listings={buildConnectorListings([], [], [implicitConnection as never], [ECHO])}
        builtIns={[]}
        onSelect={vi.fn()}
        onAdd={vi.fn()}
        onInstall={vi.fn()}
      />
    )

    expect(queryByText('Add')).not.toBeInTheDocument()
    expect(queryByText('Add another')).not.toBeInTheDocument()
    expect(getByText('no sign-in')).toBeInTheDocument()
    expect(getByText(/ready/)).toBeInTheDocument()
  })
})

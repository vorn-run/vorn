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
  capabilities: ['triggers', 'actions'],
  category: 'Data & observability',
  keywords: ['kql'],
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-kusto'] },
  triggers: [{ type: 'queryResult', label: 'Query returns a row' }],
  actions: [{ type: 'runQuery', label: 'Run a KQL query' }],
  env: [{ name: 'KUSTO_CLUSTER', required: true }]
}

const listings = (): ConnectorListing[] => buildConnectorListings([], [ADO, KUSTO], [])

const find = (id: string) => listings().find((listing) => listing.id === id)!

describe('the connector grid', () => {
  const setup = (selectedKey?: string) => {
    const onSelect = vi.fn()
    const utils = render(
      <ConnectorDirectory
        listings={listings()}
        builtIns={[]}
        selectedKey={selectedKey}
        onSelect={onSelect}
      />
    )
    return { ...utils, onSelect }
  }

  it('says what a connector offers without anything being installed', () => {
    const { getAllByText, getByText } = setup()
    // The counts are the point: a name and a blurb cannot answer "will this do
    // what I need", and both blurbs here start with the same four words.
    expect(getAllByText('1 trigger')).toHaveLength(2)
    // Only one of the two can be called from a step, which the grid now says.
    expect(getByText('1 action')).toBeInTheDocument()
  })

  it('shows the version that would be installed', () => {
    const { getByText } = setup()
    expect(getByText(/Development · v0\.1\.0/)).toBeInTheDocument()
  })

  it('finds a connector by a word that is nowhere in its name', () => {
    const { getByPlaceholderText, queryByText, getByText } = setup()
    fireEvent.change(getByPlaceholderText('Search connectors'), { target: { value: 'tfs' } })

    expect(getByText('Azure DevOps')).toBeInTheDocument()
    expect(queryByText('Azure Data Explorer')).not.toBeInTheDocument()
  })

  it('narrows to one category', () => {
    const { getByText, queryByText } = setup()
    fireEvent.click(getByText('Development'))

    expect(getByText('Azure DevOps')).toBeInTheDocument()
    expect(queryByText('Azure Data Explorer')).not.toBeInTheDocument()
  })

  it('narrows to what a workflow step can call', () => {
    const { getByText, queryByText } = setup()
    fireEvent.click(getByText('Can be called from a step'))

    expect(getByText('Azure Data Explorer')).toBeInTheDocument()
    expect(queryByText('Azure DevOps')).not.toBeInTheDocument()
  })

  it('lets a chip be clicked off, so a filter is never a trap', () => {
    const { getByText } = setup()
    fireEvent.click(getByText('Development'))
    fireEvent.click(getByText('Development'))
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

  it('reports the card that was picked', () => {
    const { getByText, onSelect } = setup()
    fireEvent.click(getByText('Azure DevOps'))
    expect(onSelect).toHaveBeenCalledWith(expect.objectContaining({ id: 'ado' }))
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
    expect(getByText('ADO_TOP')).toBeInTheDocument()
    expect(getByText('Dashed settings are optional.')).toBeInTheDocument()
  })

  it('names the package and version, so what would be installed is not a mystery', () => {
    const { getByText } = setup(find('ado'))
    expect(getByText(/@vornrun\/connector-ado · v0\.1\.0/)).toBeInTheDocument()
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
    fireEvent.click(getByText('Add connection'))
    expect(onAdd).toHaveBeenCalled()
  })

  it('is honest that reading about a connector installs nothing', () => {
    const { getByText } = setup(find('ado'))
    expect(getByText(/Nothing is installed until you add it/)).toBeInTheDocument()
  })
})

// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ConnectionGroups } from '../src/renderer/components/settings/ConnectionGroups'
import { buildConnectorListings } from '../src/renderer/lib/connector-browse'
import type { ConnectorCatalogItem, SourceConnection } from '../src/shared/types'

const ADO: ConnectorCatalogItem = {
  id: 'ado',
  name: 'Azure DevOps',
  description: 'Trigger workflows from work items.',
  packageName: '@vornrun/connector-ado',
  version: '0.1.0',
  capabilities: ['triggers'],
  category: 'Development',
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-ado'] }
}

const connection = (overrides: Partial<SourceConnection>): SourceConnection =>
  ({
    id: 'c1',
    connectorId: 'ado',
    name: 'A connection',
    filters: {},
    syncIntervalMinutes: 5,
    statusMapping: {},
    ...overrides
  }) as SourceConnection

function setup(
  connections: SourceConnection[],
  statuses: Parameters<typeof ConnectionGroups>[0]['statuses'] = []
) {
  const onAdd = vi.fn()
  const utils = render(
    <ConnectionGroups
      connections={connections}
      listings={buildConnectorListings([], [ADO], connections)}
      manifests={{}}
      statuses={statuses}
      workflows={[]}
      activity={{ busy: {}, failed: {}, run: async () => {} }}
      backfillResult={{}}
      onAdd={onAdd}
      onRun={vi.fn()}
      onBackfill={vi.fn()}
      onDelete={vi.fn()}
      onResetWorkflow={vi.fn()}
      onOpenWorkflow={vi.fn()}
      onRefresh={vi.fn()}
    />
  )
  return { ...utils, onAdd }
}

describe('connections grouped by connector', () => {
  it('puts two connections under one heading with the count beside them', () => {
    // Not on a catalog card trying to sell a third — beside the things it
    // counts.
    const { getByText } = setup([
      connection({ id: 'c1', name: 'Platform board' }),
      connection({ id: 'c2', name: 'Escalations' })
    ])

    expect(getByText('Azure DevOps')).toBeInTheDocument()
    expect(getByText('2 connections · v0.1.0')).toBeInTheDocument()
    expect(getByText('Platform board')).toBeInTheDocument()
    expect(getByText('Escalations')).toBeInTheDocument()
  })

  it('does not say "1 connections"', () => {
    const { getByText } = setup([connection({})])
    expect(getByText('1 connection · v0.1.0')).toBeInTheDocument()
  })

  it('offers another of the same kind from the heading', () => {
    const { getByText, onAdd } = setup([connection({})])
    fireEvent.click(getByText('Add another'))
    expect(onAdd).toHaveBeenCalledWith(expect.objectContaining({ id: 'ado' }))
  })

  it('still shows a connection whose connector the catalog does not list', () => {
    // A package installed by name still polls and can still fail; hiding it
    // would hide something that is running.
    const { getByText, queryByText } = setup([
      connection({ id: 'c9', connectorId: 'mystery', name: 'Something local' })
    ])

    expect(getByText('mystery')).toBeInTheDocument()
    expect(getByText('Something local')).toBeInTheDocument()
    // Nothing to open an Add form against, so nothing is offered.
    expect(queryByText('Add another')).not.toBeInTheDocument()
  })

  it('says a connector is not signed in next to its connections', () => {
    // This used to be a banner at the top of the page, nowhere near the rows
    // it was about.
    const { getByText, container } = setup(
      [connection({})],
      [{ connectorId: 'ado', authed: false, message: 'Run `az login` to sign in.' }]
    )
    expect(container.textContent).toContain('Run az login to sign in.')
    // The command is set as code, so it does not read as prose.
    expect(getByText('az login').tagName).toBe('CODE')
  })

  it('stays quiet about a connector that is signed in', () => {
    const { queryByText } = setup([connection({})], [{ connectorId: 'ado', authed: true }])
    expect(queryByText(/sign in/)).not.toBeInTheDocument()
  })

  it('renders nothing at all when there are no connections', () => {
    const { container } = setup([])
    expect(container.textContent).toBe('')
  })
})

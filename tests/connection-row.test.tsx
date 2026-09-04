// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { ConnectionRow } from '../src/renderer/components/settings/ConnectionRow'
import { rowState, type RowAction } from '../src/renderer/lib/use-row-action'
import type { ConnectorManifest, SourceConnection, WorkflowDefinition } from '../src/shared/types'

const connection = (overrides: Partial<SourceConnection> = {}): SourceConnection =>
  ({
    id: 'c1',
    connectorId: 'ado',
    name: 'Platform board',
    filters: {},
    syncIntervalMinutes: 5,
    statusMapping: {},
    ...overrides
  }) as SourceConnection

const workflow = (cron = '*/5 * * * *'): WorkflowDefinition =>
  ({
    id: 'connector:c1:workItem',
    name: 'Poll work items',
    nodes: [{ id: 't', type: 'trigger', config: { cron } }]
  }) as unknown as WorkflowDefinition

beforeEach(() => {
  ;(window as unknown as { api: unknown }).api = {}
})

// Built through the same fold the hook uses, so a key the row cannot read fails the test.
const reporting = (busy: Record<string, string>, failed: Record<string, string> = {}) => ({
  busy,
  failed,
  run: async () => {},
  state: (id: string, actions: RowAction[]) => rowState(busy, failed, id, actions)
})

function setup(props: Partial<Parameters<typeof ConnectionRow>[0]> = {}) {
  const handlers = {
    onRun: vi.fn(),
    onBackfill: vi.fn(),
    onDelete: vi.fn(),
    onResetWorkflow: vi.fn(),
    onOpenWorkflow: vi.fn(),
    onRefresh: vi.fn()
  }
  const utils = render(
    <ConnectionRow
      conn={connection()}
      seededWorkflows={[]}
      missingEvents={[]}
      activity={{ busy: {}, failed: {}, run: async () => {}, state: () => ({}) }}
      backfillResult={{}}
      {...handlers}
      {...props}
    />
  )
  return { ...utils, ...handlers }
}

describe('a configured connection', () => {
  it('names itself', () => {
    expect(setup().getByText('Platform board')).toBeInTheDocument()
  })

  it('shows the workflow doing the polling, and when', () => {
    // The mechanism is meant to be visible: the poll is a real workflow you can
    // open and change, not something hidden inside the connection.
    const { getByText } = setup({ seededWorkflows: [workflow()] })
    expect(getByText('Poll work items')).toBeInTheDocument()
    expect(getByText('· every 5 minutes')).toBeInTheDocument()
  })

  it('opens that workflow in the editor', () => {
    const { getByText, onOpenWorkflow } = setup({ seededWorkflows: [workflow()] })
    fireEvent.click(getByText('Poll work items'))
    expect(onOpenWorkflow).toHaveBeenCalledWith('connector:c1:workItem')
  })

  it('polls now rather than waiting for the next tick', () => {
    const { container, onRun } = setup({ seededWorkflows: [workflow()] })
    const run = container.querySelectorAll('button')
    fireEvent.click(run[run.length - 1])
    expect(onRun).toHaveBeenCalledWith('connector:c1:workItem', 'c1')
  })

  it('says when polling has stopped because the workflow was deleted', () => {
    // Deleting the seeded workflow silently stops the polling; without this the
    // connection looks configured and does nothing.
    const { getByText, onResetWorkflow } = setup({
      missingEvents: [{ name: 'Work items', event: 'workItem' }]
    })
    expect(getByText(/No workflow for Work items — polling disabled/)).toBeInTheDocument()
    fireEvent.click(getByText('Reset default workflow'))
    expect(onResetWorkflow).toHaveBeenCalledWith('c1', 'workItem')
  })

  it('imports what the cron cursor already passed over', () => {
    const { getAllByRole, onBackfill } = setup()
    fireEvent.click(getAllByRole('button')[0])
    expect(onBackfill).toHaveBeenCalledWith('c1')
  })

  it('reports what a backfill actually did', () => {
    const { getByText } = setup({
      backfillResult: { c1: { imported: 3, updated: 2 } }
    })
    expect(getByText('+3 imported, 2 updated')).toBeInTheDocument()
  })

  it('reports a backfill that failed rather than looking like it worked', () => {
    const { getByText } = setup({
      backfillResult: { c1: { imported: 0, updated: 0, error: '403 from the API' } }
    })
    expect(getByText('403 from the API')).toBeInTheDocument()
  })

  it('shows the last sync and any error from it', () => {
    const { getByText } = setup({
      conn: connection({
        lastSyncAt: '2026-08-05T12:00:00.000Z',
        lastSyncError: 'sign-in expired'
      })
    })
    expect(getByText(/Last synced/)).toBeInTheDocument()
    expect(getByText('sign-in expired')).toBeInTheDocument()
  })

  it('shows the filters it was configured with', () => {
    const { getByText } = setup({ conn: connection({ filters: { project: 'Platform' } }) })
    expect(getByText('project: Platform')).toBeInTheDocument()
  })

  it('masks a filter the connector declared as a password', () => {
    // Filters are rendered verbatim, so a secret among them would otherwise be
    // printed on the settings page.
    const manifest = {
      auth: [{ key: 'token', label: 'Token', type: 'password' }]
    } as unknown as ConnectorManifest
    const { getByText } = setup({
      conn: connection({ filters: { token: 'super-secret' } }),
      manifest
    })
    expect(getByText('token: ••••••')).toBeInTheDocument()
  })

  it('truncates a filter too long to sit on one line', () => {
    const long = 'x'.repeat(100)
    const { getByText } = setup({ conn: connection({ filters: { query: long } }) })
    expect(getByText(/query: x+…$/)).toBeInTheDocument()
  })

  it('leaves the MCP tool list out of the filter tags', () => {
    // It is rendered as its own panel below; as a tag it would be a wall of
    // JSON.
    const { queryByText } = setup({
      conn: connection({ filters: { discoveredTools: '[{"name":"a"}]' } })
    })
    expect(queryByText(/discoveredTools/)).not.toBeInTheDocument()
  })

  it('deletes on request', () => {
    const { getAllByRole, onDelete } = setup()
    const buttons = getAllByRole('button')
    fireEvent.click(buttons[1])
    expect(onDelete).toHaveBeenCalledWith('c1')
  })
})

describe('what a connection says while its own actions run', () => {
  it('says it is importing, and holds the button, while a backfill runs', () => {
    const { getByText, container } = setup({ activity: reporting({ 'backfill:c1': 'Importing…' }) })

    expect(getByText('Importing…')).toBeInTheDocument()
    const backfill = container.querySelectorAll('button')[0]
    expect(backfill).toBeDisabled()
  })

  it('says it is removing while a delete runs', () => {
    const { getByText } = setup({ activity: reporting({ 'delete:c1': 'Removing…' }) })

    expect(getByText('Removing…')).toBeInTheDocument()
  })

  it('holds the poll button for the workflow being polled, not its neighbour', () => {
    const { container } = setup({
      seededWorkflows: [workflow()],
      activity: reporting({ 'run:connector:c1:workItem': 'Polling…' })
    })

    const poll = Array.from(container.querySelectorAll('button')).at(-1)
    expect(poll).toBeDisabled()
  })

  it('keeps a refusal on the row it was pressed on', () => {
    const { getByText } = setup({
      activity: reporting({}, { 'backfill:c1': 'The connector never answered' })
    })

    expect(getByText('The connector never answered')).toBeInTheDocument()
  })
})

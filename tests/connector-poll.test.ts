import { describe, it, expect, vi, beforeEach } from 'vitest'
import type {
  WorkflowDefinition,
  ConnectorPollTriggerConfig,
  SourceConnection,
  PollResult
} from '../packages/shared/src/types'

// vi.mock() factories are hoisted to the top of the file; any variables they
// reference must be hoisted too. vi.hoisted() is the blessed way to share
// spies between the mock factory and the test body.
const {
  loadConfigMock,
  dbGetSourceConnectionMock,
  dbGetConnectorPollCursorMock,
  dbRecordConnectorPollPageMock,
  dbRecordConnectorPollErrorMock,
  connectorGetMock
} = vi.hoisted(() => ({
  loadConfigMock: vi.fn(),
  dbGetSourceConnectionMock: vi.fn(),
  dbGetConnectorPollCursorMock: vi.fn(),
  dbRecordConnectorPollPageMock: vi.fn(),
  dbRecordConnectorPollErrorMock: vi.fn(),
  connectorGetMock: vi.fn()
}))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('../packages/server/src/database', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return {
    ...actual,
    dbGetWorkflow: (id: string) =>
      (loadConfigMock() as { workflows?: WorkflowDefinition[] } | undefined)?.workflows?.find(
        (w) => w.id === id
      ) ?? null,
    dbGetSourceConnection: dbGetSourceConnectionMock,
    dbGetConnectorPollCursor: dbGetConnectorPollCursorMock,
    dbRecordConnectorPollPage: dbRecordConnectorPollPageMock,
    dbRecordConnectorPollError: dbRecordConnectorPollErrorMock
  }
})
vi.mock('../packages/server/src/connectors', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return {
    ...actual,
    connectorRegistry: { get: connectorGetMock },
    applyDecryptedCreds: (conn: { filters: Record<string, unknown> }) => ({ ...conn.filters })
  }
})
// Import after mocks are set up.
import { pollConnector } from '../packages/server/src/connector-poll'

function makeConn(overrides: Partial<SourceConnection> = {}): SourceConnection {
  return {
    id: 'conn-1',
    connectorId: 'github',
    name: 'owner/repo',
    filters: { owner: 'owner', repo: 'repo' },
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-04-24T00:00:00Z',
    ...overrides
  }
}

function makePollWorkflow(id = 'wf-1'): WorkflowDefinition {
  const trigger: ConnectorPollTriggerConfig = {
    triggerType: 'connectorPoll',
    connectionId: 'conn-1',
    event: 'issueCreated',
    cron: '*/5 * * * *'
  }
  return {
    id,
    name: 'Test Poll',
    icon: 'Plug',
    iconColor: '#64748b',
    enabled: true,
    nodes: [
      { id: 'trigger-1', type: 'trigger', label: 't', config: trigger, position: { x: 0, y: 0 } }
    ],
    edges: []
  }
}

beforeEach(() => {
  loadConfigMock.mockReset()
  dbGetSourceConnectionMock.mockReset()
  dbGetConnectorPollCursorMock.mockReset()
  dbGetConnectorPollCursorMock.mockReturnValue(undefined)
  dbRecordConnectorPollPageMock.mockReset()
  dbRecordConnectorPollErrorMock.mockReset()
  connectorGetMock.mockReset()
})

/**
 * `connector:poll`: vornd fires a connector-poll schedule and asks this server,
 * which holds the connectors, to fetch the new items into the inbox. vornd
 * runs them from there.
 */
describe('polling a connection into the inbox', () => {
  it('advances cursor and updates lastSyncAt on a successful poll', async () => {
    const wf = makePollWorkflow('wf-ok')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn())

    const pollResult: PollResult = {
      events: [
        {
          id: '1',
          type: 'issueCreated',
          data: { externalId: '1', title: 'X' },
          timestamp: '2026-04-24T10:00:00Z'
        }
      ],
      nextCursor: '2026-04-24T10:05:00Z'
    }
    connectorGetMock.mockReturnValue({ poll: vi.fn().mockResolvedValue(pollResult) })

    await pollConnector('wf-ok')

    expect(dbRecordConnectorPollPageMock).toHaveBeenCalledWith(
      expect.objectContaining({
        workflowId: 'wf-ok',
        connectionId: 'conn-1',
        cursor: '2026-04-24T10:05:00Z'
      })
    )
  })

  it('records lastSyncError and skips emitting when poll throws', async () => {
    const wf = makePollWorkflow('wf-err')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn())
    connectorGetMock.mockReturnValue({
      poll: vi.fn().mockRejectedValue(new Error('gh network down'))
    })

    await pollConnector('wf-err')

    expect(dbRecordConnectorPollErrorMock).toHaveBeenCalledWith(
      expect.objectContaining({
        workflowId: 'wf-err',
        connectionId: 'conn-1',
        error: 'gh network down'
      })
    )
    expect(dbRecordConnectorPollPageMock).not.toHaveBeenCalled()
  })

  it('drains every bounded remote page before dispatching the inbox', async () => {
    const wf = makePollWorkflow('wf-pages')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn())
    const poll = vi
      .fn()
      .mockResolvedValueOnce({
        events: [{ id: '1', type: 'issueCreated', data: { title: 'A' }, timestamp: 't1' }],
        nextCursor: 'page-2',
        hasMore: true
      })
      .mockResolvedValueOnce({
        events: [{ id: '2', type: 'issueCreated', data: { title: 'B' }, timestamp: 't2' }],
        nextCursor: 'caught-up',
        hasMore: false
      })
    connectorGetMock.mockReturnValue({ poll })

    await pollConnector('wf-pages')

    expect(poll).toHaveBeenNthCalledWith(1, 'issueCreated', expect.anything(), undefined)
    expect(poll).toHaveBeenNthCalledWith(2, 'issueCreated', expect.anything(), 'page-2')
    expect(dbRecordConnectorPollPageMock).toHaveBeenCalledTimes(2)
    expect(dbRecordConnectorPollPageMock.mock.calls[1][0]).toMatchObject({
      cursor: 'caught-up',
      events: [expect.objectContaining({ eventId: '2' })]
    })
  })

  it('uses the workflow cursor instead of another subscription’s connection cursor', async () => {
    const wf = makePollWorkflow('wf-own-cursor')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn({ syncCursor: 'legacy-shared' }))
    dbGetConnectorPollCursorMock.mockReturnValue('workflow-specific')
    const poll = vi.fn().mockResolvedValue({ events: [], nextCursor: 'next' })
    connectorGetMock.mockReturnValue({ poll })

    await pollConnector('wf-own-cursor')

    expect(poll).toHaveBeenCalledWith('issueCreated', expect.anything(), 'workflow-specific')
  })

  it('rejects hasMore when the connector does not advance its cursor', async () => {
    const wf = makePollWorkflow('wf-stuck')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn({ syncCursor: 'same' }))
    dbGetConnectorPollCursorMock.mockReturnValue('same')
    connectorGetMock.mockReturnValue({
      poll: vi.fn().mockResolvedValue({ events: [], nextCursor: 'same', hasMore: true })
    })

    await pollConnector('wf-stuck')

    expect(dbRecordConnectorPollPageMock).not.toHaveBeenCalled()
    expect(dbRecordConnectorPollErrorMock).toHaveBeenCalledWith(
      expect.objectContaining({ workflowId: 'wf-stuck', error: expect.stringContaining('hasMore') })
    )
  })

  it('skips silently when the connection was deleted between scheduling and firing', async () => {
    const wf = makePollWorkflow('wf-gone')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(null)

    await pollConnector('wf-gone')

    expect(dbRecordConnectorPollPageMock).not.toHaveBeenCalled()
    expect(connectorGetMock).not.toHaveBeenCalled()
  })

  it('skips silently when the connector has no poll() method', async () => {
    const wf = makePollWorkflow('wf-nopoll')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn())
    connectorGetMock.mockReturnValue({}) // no poll

    await pollConnector('wf-nopoll')

    expect(dbRecordConnectorPollPageMock).not.toHaveBeenCalled()
  })

  // --- A connector that runs a child per connection polls with the connection itself ---

  function makeGenericPollWorkflow(id: string, event: string): WorkflowDefinition {
    const trigger: ConnectorPollTriggerConfig = {
      triggerType: 'connectorPoll',
      connectionId: 'conn-1',
      event,
      cron: '*/5 * * * *'
    }
    return {
      id,
      name: 'Poll',
      icon: 'Plug',
      iconColor: '#64748b',
      enabled: true,
      nodes: [
        { id: 'trigger-1', type: 'trigger', label: 't', config: trigger, position: { x: 0, y: 0 } }
      ],
      edges: []
    }
  }

  /** The connector hands back this pager, or this reason it has nothing to poll. */
  const pagerFor = (pager: ((cursor?: string) => Promise<PollResult>) | string) => {
    const pollConnection = vi.fn(() => pager)
    connectorGetMock.mockReturnValue({ pollConnection })
    return pollConnection
  }

  const sdkConn = () => makeConn({ connectorId: 'sdk', name: 'Substack' })

  const fire = async (workflowId: string): Promise<void> => {
    await pollConnector(workflowId)
  }

  it('hands the whole connection and the event to a connector that polls per connection', async () => {
    loadConfigMock.mockReturnValue({ workflows: [makeGenericPollWorkflow('wf-conn', 'mcpPoll')] })
    dbGetSourceConnectionMock.mockReturnValue(sdkConn())
    const page = vi.fn().mockResolvedValue({
      events: [
        { id: 'p1', type: 'mcpPoll', data: { externalId: 'p1', title: 'P' }, timestamp: 't1' }
      ],
      nextCursor: 'c1',
      hasMore: false
    })
    const pollConnection = pagerFor(page)

    await fire('wf-conn')

    expect(pollConnection).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'conn-1', connectorId: 'sdk' }),
      'mcpPoll'
    )
    expect(page).toHaveBeenCalledWith(undefined)
    expect(dbRecordConnectorPollPageMock).toHaveBeenCalledWith(
      expect.objectContaining({ workflowId: 'wf-conn', connectorId: 'sdk', cursor: 'c1' })
    )
  })

  it('skips a connection whose connector says it has nothing to poll', async () => {
    loadConfigMock.mockReturnValue({ workflows: [makeGenericPollWorkflow('wf-none', 'mcpPoll')] })
    dbGetSourceConnectionMock.mockReturnValue(sdkConn())
    pagerFor('has no trigger to poll')

    await fire('wf-none')

    expect(dbRecordConnectorPollPageMock).not.toHaveBeenCalled()
    expect(dbRecordConnectorPollErrorMock).not.toHaveBeenCalled()
  })

  it('keeps polling a connection while it says there is more', async () => {
    loadConfigMock.mockReturnValue({ workflows: [makeGenericPollWorkflow('wf-pages', 'mcpPoll')] })
    dbGetSourceConnectionMock.mockReturnValue(sdkConn())
    const page = vi
      .fn()
      .mockResolvedValueOnce({ events: [], nextCursor: 'p1', hasMore: true })
      .mockResolvedValueOnce({ events: [], nextCursor: 'p2', hasMore: false })
    pagerFor(page)

    await fire('wf-pages')

    expect(page.mock.calls.map((call) => call[0])).toEqual([undefined, 'p1'])
    expect(dbRecordConnectorPollPageMock).toHaveBeenCalledTimes(2)
  })

  it('records a pack built for an older Vorn as the sync error, saying how to fix it', async () => {
    loadConfigMock.mockReturnValue({ workflows: [makeGenericPollWorkflow('wf-old', 'mcpPoll')] })
    dbGetSourceConnectionMock.mockReturnValue(sdkConn())
    const outdated =
      'Substack was built for an older Vorn. Update it in Settings → Connectors, or rebuild it with @vornrun/connector-sdk 0.7.1-beta.3 or later.'
    pagerFor(vi.fn().mockRejectedValue(new Error(outdated)))

    await fire('wf-old')

    expect(dbRecordConnectorPollErrorMock).toHaveBeenCalledWith(
      expect.objectContaining({ workflowId: 'wf-old', connectionId: 'conn-1', error: outdated })
    )
    expect(dbRecordConnectorPollPageMock).not.toHaveBeenCalled()
  })
})

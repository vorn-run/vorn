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
  dbGetWorkflowRunByConnectorInboxIdMock,
  dbRecordConnectorPollPageMock,
  dbRecordConnectorPollErrorMock,
  dbClaimConnectorInboxMock,
  dbCountActiveConnectorInboxLeasesMock,
  dbCompleteConnectorInboxMock,
  dbRetryConnectorInboxMock,
  dbDeferConnectorInboxMock,
  dbRenewConnectorInboxLeaseMock,
  clientRegistryMock,
  connectorGetMock
} = vi.hoisted(() => ({
  loadConfigMock: vi.fn(),
  dbGetSourceConnectionMock: vi.fn(),
  dbGetConnectorPollCursorMock: vi.fn(),
  dbGetWorkflowRunByConnectorInboxIdMock: vi.fn(),
  dbRecordConnectorPollPageMock: vi.fn(),
  dbRecordConnectorPollErrorMock: vi.fn(),
  dbClaimConnectorInboxMock: vi.fn(),
  dbCountActiveConnectorInboxLeasesMock: vi.fn(),
  dbCompleteConnectorInboxMock: vi.fn(),
  dbRetryConnectorInboxMock: vi.fn(),
  dbDeferConnectorInboxMock: vi.fn(),
  dbRenewConnectorInboxLeaseMock: vi.fn(),
  clientRegistryMock: { size: 1 },
  connectorGetMock: vi.fn()
}))

const runScheduled = vi.hoisted(() => vi.fn(async (_workflowId: string, _inputs?: unknown) => {}))
const runConnectorItem = vi.hoisted(() => vi.fn(async (_event: unknown) => {}))
vi.mock('../packages/server/src/workflows/dispatch', () => ({ runScheduled, runConnectorItem }))
vi.mock('../packages/server/src/workflows/engine', () => ({ stopWorkflowRun: vi.fn() }))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node-cron', () => ({
  default: {
    schedule: vi.fn(() => ({ stop: vi.fn() })),
    validate: vi.fn(() => true)
  }
}))
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: { loadConfig: loadConfigMock, saveConfig: vi.fn(), notifyChanged: vi.fn() }
}))
vi.mock('../packages/server/src/broadcast', () => ({
  clientRegistry: clientRegistryMock
}))
vi.mock('../packages/server/src/database', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return {
    ...actual,
    dbGetSourceConnection: dbGetSourceConnectionMock,
    dbGetConnectorPollCursor: dbGetConnectorPollCursorMock,
    dbGetWorkflowRunByConnectorInboxId: dbGetWorkflowRunByConnectorInboxIdMock,
    dbRecordConnectorPollPage: dbRecordConnectorPollPageMock,
    dbRecordConnectorPollError: dbRecordConnectorPollErrorMock,
    dbClaimConnectorInbox: dbClaimConnectorInboxMock,
    dbCountActiveConnectorInboxLeases: dbCountActiveConnectorInboxLeasesMock,
    dbCompleteConnectorInbox: dbCompleteConnectorInboxMock,
    dbRetryConnectorInbox: dbRetryConnectorInboxMock,
    dbDeferConnectorInbox: dbDeferConnectorInboxMock,
    dbRenewConnectorInboxLease: dbRenewConnectorInboxLeaseMock
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
import { scheduler } from '../packages/server/src/scheduler'

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
  dbGetWorkflowRunByConnectorInboxIdMock.mockReset()
  dbGetWorkflowRunByConnectorInboxIdMock.mockReturnValue(null)
  dbRecordConnectorPollPageMock.mockReset()
  dbRecordConnectorPollErrorMock.mockReset()
  dbClaimConnectorInboxMock.mockReset()
  dbClaimConnectorInboxMock.mockReturnValue([])
  dbCountActiveConnectorInboxLeasesMock.mockReset()
  dbCountActiveConnectorInboxLeasesMock.mockReturnValue(0)
  dbCompleteConnectorInboxMock.mockReset()
  dbRetryConnectorInboxMock.mockReset()
  dbDeferConnectorInboxMock.mockReset()
  dbRenewConnectorInboxLeaseMock.mockReset()
  clientRegistryMock.size = 1
  runScheduled.mockClear()
  runConnectorItem.mockClear()
  connectorGetMock.mockReset()
})

describe('scheduler.triggerWorkflow for connectorPoll', () => {
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

    scheduler.triggerWorkflow('wf-ok')
    // dispatchConnectorPoll is async; flush microtasks.
    await new Promise((r) => setImmediate(r))

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

    const emitted: Array<[string, unknown]> = []
    const listener = (ch: string, payload: unknown): void => {
      emitted.push([ch, payload])
    }
    scheduler.on('client-message', listener)

    scheduler.triggerWorkflow('wf-err')
    await new Promise((r) => setImmediate(r))
    scheduler.off('client-message', listener)

    // Scheduler should record the error without emitting a bounce event.
    expect(dbRecordConnectorPollErrorMock).toHaveBeenCalledWith(
      expect.objectContaining({
        workflowId: 'wf-err',
        connectionId: 'conn-1',
        error: 'gh network down'
      })
    )
    expect(emitted.length).toBe(0)
  })

  it('runs one workflow per item when a poll yields several', async () => {
    const wf = makePollWorkflow('wf-items')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn())
    connectorGetMock.mockReturnValue({
      poll: vi.fn().mockResolvedValue({
        events: [
          {
            id: '1',
            type: 'issueCreated',
            data: { externalId: '1', title: 'A', url: 'https://u/1' },
            timestamp: 't1'
          },
          {
            id: '2',
            type: 'issueCreated',
            data: { externalId: '2', title: 'B', url: 'https://u/2' },
            timestamp: 't2'
          }
        ],
        nextCursor: 'now'
      })
    })
    dbClaimConnectorInboxMock.mockReturnValue([
      {
        id: 11,
        leaseToken: 'lease-11',
        workflowId: 'wf-items',
        connectorItem: {
          connectionId: 'conn-1',
          connectorId: 'github',
          externalId: '1',
          title: 'A',
          raw: {}
        }
      },
      {
        id: 12,
        leaseToken: 'lease-12',
        workflowId: 'wf-items',
        connectorItem: {
          connectionId: 'conn-1',
          connectorId: 'github',
          externalId: '2',
          title: 'B',
          raw: {}
        }
      }
    ])

    scheduler.triggerWorkflow('wf-items')
    await new Promise((r) => setImmediate(r))

    const emitted = runConnectorItem.mock.calls.map(
      ([event]) =>
        event as {
          workflowId: string
          connectorItem?: { externalId?: string; title?: string }
          connectorInboxId?: number
          connectorInboxLeaseToken?: string
        }
    )
    expect(emitted).toHaveLength(2)
    expect(emitted[0].connectorItem).toMatchObject({ externalId: '1', title: 'A' })
    expect(emitted[1].connectorItem).toMatchObject({ externalId: '2', title: 'B' })
    expect(emitted.map((event) => event.connectorInboxId)).toEqual([11, 12])
    expect(emitted.map((event) => event.connectorInboxLeaseToken)).toEqual(['lease-11', 'lease-12'])
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

    scheduler.triggerWorkflow('wf-pages')
    await new Promise((r) => setImmediate(r))

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

    scheduler.triggerWorkflow('wf-own-cursor')
    await new Promise((r) => setImmediate(r))

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

    scheduler.triggerWorkflow('wf-stuck')
    await new Promise((r) => setImmediate(r))

    expect(dbRecordConnectorPollPageMock).not.toHaveBeenCalled()
    expect(dbRecordConnectorPollErrorMock).toHaveBeenCalledWith(
      expect.objectContaining({ workflowId: 'wf-stuck', error: expect.stringContaining('hasMore') })
    )
  })

  it('acknowledges success and backs off workflow failures', () => {
    scheduler.completeConnectorInbox(41, 'lease-41', 'processed')
    expect(dbCompleteConnectorInboxMock).toHaveBeenCalledWith(41, 'lease-41', expect.any(String))

    scheduler.completeConnectorInbox(42, 'lease-42', 'retry', 'agent failed')
    expect(dbRetryConnectorInboxMock).toHaveBeenCalledWith(
      expect.objectContaining({ id: 42, leaseToken: 'lease-42', error: 'agent failed' })
    )

    scheduler.completeConnectorInbox(43, 'lease-43', 'defer')
    expect(dbDeferConnectorInboxMock).toHaveBeenCalledWith(43, 'lease-43', expect.any(String))
  })

  it('claims inbox rows with nobody connected, because it runs them itself', () => {
    clientRegistryMock.size = 0
    scheduler.deliverPendingConnectorInbox()
    expect(dbClaimConnectorInboxMock).toHaveBeenCalled()
  })

  it('claims only the remaining global delivery capacity', () => {
    dbCountActiveConnectorInboxLeasesMock.mockReturnValue(49)

    scheduler.deliverPendingConnectorInbox()

    expect(dbClaimConnectorInboxMock).toHaveBeenCalledWith(expect.objectContaining({ limit: 1 }))
  })

  it('redelivers a leased row with its persisted running execution', () => {
    dbClaimConnectorInboxMock.mockReturnValue([
      {
        id: 51,
        leaseToken: 'lease-51',
        workflowId: 'wf-items',
        connectorItem: {
          connectionId: 'conn-1',
          connectorId: 'github',
          externalId: '51',
          title: 'A',
          raw: {}
        }
      }
    ])
    dbGetWorkflowRunByConnectorInboxIdMock.mockReturnValue({
      runId: 'run-51',
      workflowId: 'wf-items',
      startedAt: '2026-04-24T10:00:00Z',
      status: 'running',
      connectorInboxId: 51,
      nodeStates: [{ nodeId: 'approval', status: 'waiting' }]
    })
    scheduler.deliverPendingConnectorInbox()

    expect(runConnectorItem).toHaveBeenCalledWith(
      expect.objectContaining({
        connectorInboxLeaseToken: 'lease-51',
        existingExecution: expect.objectContaining({ runId: 'run-51' })
      })
    )
  })

  it('finishes a redelivered row whose persisted run already succeeded', () => {
    dbClaimConnectorInboxMock.mockReturnValue([
      {
        id: 52,
        leaseToken: 'lease-52',
        workflowId: 'wf-items',
        connectorItem: {
          connectionId: 'conn-1',
          connectorId: 'github',
          externalId: '52',
          title: 'B',
          raw: {}
        }
      }
    ])
    dbGetWorkflowRunByConnectorInboxIdMock.mockReturnValue({
      runId: 'run-52',
      workflowId: 'wf-items',
      startedAt: '2026-04-24T10:00:00Z',
      completedAt: '2026-04-24T10:01:00Z',
      status: 'success',
      connectorInboxId: 52,
      connectorInboxDisposition: 'processed',
      nodeStates: []
    })
    const listener = vi.fn()
    scheduler.on('client-message', listener)

    scheduler.deliverPendingConnectorInbox()

    scheduler.off('client-message', listener)
    expect(dbCompleteConnectorInboxMock).toHaveBeenCalledWith(52, 'lease-52', expect.any(String))
    expect(listener).not.toHaveBeenCalled()
  })

  it('skips silently when the connection was deleted between scheduling and firing', async () => {
    const wf = makePollWorkflow('wf-gone')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(null)

    scheduler.triggerWorkflow('wf-gone')
    await new Promise((r) => setImmediate(r))

    expect(dbRecordConnectorPollPageMock).not.toHaveBeenCalled()
    expect(connectorGetMock).not.toHaveBeenCalled()
  })

  it('skips silently when the connector has no poll() method', async () => {
    const wf = makePollWorkflow('wf-nopoll')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn())
    connectorGetMock.mockReturnValue({}) // no poll

    scheduler.triggerWorkflow('wf-nopoll')
    await new Promise((r) => setImmediate(r))

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
    scheduler.triggerWorkflow(workflowId)
    await new Promise((r) => setImmediate(r))
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

  it('polls one at a time, however often it is asked', async () => {
    // A poll calls the connector and writes the inbox here, before any run
    // claim exists, so two fires would otherwise read the same cursor twice.
    const wf = makePollWorkflow('wf-serial')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn())
    let release: () => void = () => {}
    const inFlight = new Promise<void>((resolve) => {
      release = resolve
    })
    const poll = vi.fn().mockImplementation(() => inFlight.then(() => ({ events: [] })))
    connectorGetMock.mockReturnValue({ poll })

    scheduler.triggerWorkflow('wf-serial')
    scheduler.triggerWorkflow('wf-serial')
    await new Promise((r) => setImmediate(r))
    expect(poll).toHaveBeenCalledTimes(1)

    release()
    await new Promise((r) => setImmediate(r))
    await new Promise((r) => setImmediate(r))

    // The next ask, once the first has finished, polls again.
    scheduler.triggerWorkflow('wf-serial')
    await new Promise((r) => setImmediate(r))
    expect(poll).toHaveBeenCalledTimes(2)
  })
})

describe('the count that decides whether this server may leave', () => {
  // `serverSideScheduleCount` is the only schedule kind that vetoes an idle
  // shutdown, because a connector poll is the only one this server performs
  // itself. A recurring or one-off trigger is executed by a renderer, so waiting
  // for one would keep a promise by dropping the run.
  const recurring = (id: string): WorkflowDefinition => {
    const wf = makePollWorkflow(id)
    wf.nodes[0].config = {
      triggerType: 'recurring',
      cron: '*/5 * * * *'
    } as unknown as ConnectorPollTriggerConfig
    return wf
  }

  beforeEach(() => {
    scheduler.stopAll()
  })

  it('counts every armed schedule, because it can act on all of them', () => {
    scheduler.syncSchedules([makePollWorkflow('poll-a'), recurring('cron-a')])
    expect(scheduler.serverSideScheduleCount()).toBe(2)
  })

  it('keeps counting a workflow whose trigger changes kind', () => {
    // Both kinds are armed here and both are acted on, so the count holds
    // steady across a change that used to move a workflow in and out of it.
    scheduler.syncSchedules([recurring('wf-x')])
    expect(scheduler.serverSideScheduleCount()).toBe(1)

    scheduler.syncSchedules([makePollWorkflow('wf-x')])
    expect(scheduler.serverSideScheduleCount()).toBe(1)

    scheduler.syncSchedules([recurring('wf-x')])
    expect(scheduler.serverSideScheduleCount()).toBe(1)
  })

  it('drops the count when the workflow is disabled', () => {
    scheduler.syncSchedules([makePollWorkflow('wf-y')])
    expect(scheduler.serverSideScheduleCount()).toBe(1)

    const disabled = makePollWorkflow('wf-y')
    disabled.enabled = false
    scheduler.syncSchedules([disabled])
    expect(scheduler.serverSideScheduleCount()).toBe(0)
  })
})

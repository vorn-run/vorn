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
  connectorGetMock,
  pollMcpConnectionMock
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
  connectorGetMock: vi.fn(),
  pollMcpConnectionMock: vi.fn()
}))

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
// Keep the real MCP_CONNECTOR_ID / MCP_POLL_EVENT constants; stub only the poll.
vi.mock('../packages/server/src/connectors/mcp', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, pollMcpConnection: pollMcpConnectionMock }
})

// Import after mocks are set up.
import { scheduler } from '../packages/server/src/scheduler'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'

// The execution lock writes a file; point it at an empty tmp dir per-test so
// no two tests collide on filenames.
const LOCK_DIR = path.join(os.homedir(), '.vorn')
try {
  fs.mkdirSync(LOCK_DIR, { recursive: true })
} catch {
  /* ignore */
}

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
  connectorGetMock.mockReset()
  pollMcpConnectionMock.mockReset()
  // Clean up any stale lock files from previous tests to avoid the minute-key
  // dedup blocking subsequent triggerWorkflow() calls in the same minute.
  try {
    for (const f of fs.readdirSync(LOCK_DIR)) {
      if (f.startsWith('scheduler-wf-') && f.endsWith('.lock')) {
        fs.unlinkSync(path.join(LOCK_DIR, f))
      }
    }
  } catch {
    /* ignore */
  }
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

  it('emits per-item SCHEDULER_EXECUTE events with connectorItem when poll yields items', async () => {
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

    const emitted: Array<{
      workflowId: string
      connectorItem?: unknown
      connectorInboxId?: number
      connectorInboxLeaseToken?: string
    }> = []
    const listener = (
      _ch: string,
      payload: { workflowId: string; connectorItem?: unknown }
    ): void => {
      emitted.push(payload)
    }
    scheduler.on('client-message', listener)

    scheduler.triggerWorkflow('wf-items')
    await new Promise((r) => setImmediate(r))
    scheduler.off('client-message', listener)

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

  it('leaves inbox rows unclaimed while no renderer is connected', () => {
    clientRegistryMock.size = 0
    scheduler.deliverPendingConnectorInbox()
    expect(dbClaimConnectorInboxMock).not.toHaveBeenCalled()
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
    const listener = vi.fn()
    scheduler.on('client-message', listener)

    scheduler.deliverPendingConnectorInbox()

    scheduler.off('client-message', listener)
    expect(listener).toHaveBeenCalledWith(
      'scheduler:execute',
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

  // --- MCP connections are routed through pollMcpConnection, not connector.poll ---

  function makeMcpPollWorkflow(id: string, event: string): WorkflowDefinition {
    const trigger: ConnectorPollTriggerConfig = {
      triggerType: 'connectorPoll',
      connectionId: 'conn-1',
      event,
      cron: '*/5 * * * *'
    }
    return {
      id,
      name: 'MCP Poll',
      icon: 'Plug',
      iconColor: '#64748b',
      enabled: true,
      nodes: [
        { id: 'trigger-1', type: 'trigger', label: 't', config: trigger, position: { x: 0, y: 0 } }
      ],
      edges: []
    }
  }

  it('routes an mcpPoll event through pollMcpConnection and advances the cursor', async () => {
    const wf = makeMcpPollWorkflow('wf-mcp', 'mcpPoll')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(
      makeConn({ connectorId: 'mcp', filters: { pollTool: 'list' }, syncCursor: 'c0' })
    )
    connectorGetMock.mockReturnValue({}) // MCP has no generic poll()
    pollMcpConnectionMock.mockResolvedValue({
      events: [
        { id: '1', type: 'mcpPoll', data: { externalId: '1', title: 'X' }, timestamp: 't1' }
      ],
      nextCursor: 't1'
    })

    scheduler.triggerWorkflow('wf-mcp')
    await new Promise((r) => setImmediate(r))

    // pollMcpConnection called with the full connection + cursor.
    expect(pollMcpConnectionMock).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'conn-1', connectorId: 'mcp' }),
      undefined
    )
    expect(dbRecordConnectorPollPageMock).toHaveBeenCalledWith(
      expect.objectContaining({ workflowId: 'wf-mcp', connectionId: 'conn-1', cursor: 't1' })
    )
  })

  it('skips an MCP connection whose event is not mcpPoll', async () => {
    const wf = makeMcpPollWorkflow('wf-mcp-bad', 'issueCreated')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(
      makeConn({ connectorId: 'mcp', filters: { pollTool: 'list' } })
    )
    connectorGetMock.mockReturnValue({})

    scheduler.triggerWorkflow('wf-mcp-bad')
    await new Promise((r) => setImmediate(r))

    expect(pollMcpConnectionMock).not.toHaveBeenCalled()
    expect(dbRecordConnectorPollPageMock).not.toHaveBeenCalled()
  })
})

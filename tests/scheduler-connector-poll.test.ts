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
  dbUpdateSourceConnectionMock,
  connectorGetMock,
  pollMcpConnectionMock
} = vi.hoisted(() => ({
  loadConfigMock: vi.fn(),
  dbGetSourceConnectionMock: vi.fn(),
  dbUpdateSourceConnectionMock: vi.fn(),
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
vi.mock('../packages/server/src/database', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return {
    ...actual,
    dbGetSourceConnection: dbGetSourceConnectionMock,
    dbUpdateSourceConnection: dbUpdateSourceConnectionMock
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
  dbUpdateSourceConnectionMock.mockReset()
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

    expect(dbUpdateSourceConnectionMock).toHaveBeenCalledWith(
      'conn-1',
      expect.objectContaining({ syncCursor: '2026-04-24T10:05:00Z', lastSyncError: undefined })
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
    expect(dbUpdateSourceConnectionMock).toHaveBeenCalledWith(
      'conn-1',
      expect.objectContaining({ lastSyncError: 'gh network down' })
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

    const emitted: Array<{ workflowId: string; connectorItem?: unknown }> = []
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
  })

  it('skips silently when the connection was deleted between scheduling and firing', async () => {
    const wf = makePollWorkflow('wf-gone')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(null)

    scheduler.triggerWorkflow('wf-gone')
    await new Promise((r) => setImmediate(r))

    expect(dbUpdateSourceConnectionMock).not.toHaveBeenCalled()
    expect(connectorGetMock).not.toHaveBeenCalled()
  })

  it('skips silently when the connector has no poll() method', async () => {
    const wf = makePollWorkflow('wf-nopoll')
    loadConfigMock.mockReturnValue({ workflows: [wf] })
    dbGetSourceConnectionMock.mockReturnValue(makeConn())
    connectorGetMock.mockReturnValue({}) // no poll

    scheduler.triggerWorkflow('wf-nopoll')
    await new Promise((r) => setImmediate(r))

    expect(dbUpdateSourceConnectionMock).not.toHaveBeenCalled()
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
      'c0'
    )
    expect(dbUpdateSourceConnectionMock).toHaveBeenCalledWith(
      'conn-1',
      expect.objectContaining({ syncCursor: 't1' })
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
    expect(dbUpdateSourceConnectionMock).not.toHaveBeenCalled()
  })
})

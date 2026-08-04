import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type {
  ConnectorItemContext,
  SourceConnection,
  WorkflowDefinition
} from '../packages/shared/src/types'
import {
  dbClaimConnectorInbox,
  dbCompleteConnectorInbox,
  dbCountActiveConnectorInboxLeases,
  dbDeferConnectorInbox,
  dbGetConnectorPollCursor,
  dbGetWorkflowRunByConnectorInboxId,
  dbInsertSourceConnection,
  dbInsertWorkflow,
  dbListSourceConnections,
  dbRecordConnectorPollError,
  dbRecordConnectorPollPage,
  dbReleaseConnectorInboxLeases,
  dbRenewConnectorInboxLease,
  dbRetryConnectorInbox,
  initTestDatabase,
  loadConfig,
  saveConfig,
  saveWorkflowRun
} from '../packages/server/src/database'

let teardown: () => void

const workflow = (id: string): WorkflowDefinition => ({
  id,
  name: id,
  icon: 'Plug',
  iconColor: '#fff',
  enabled: true,
  nodes: [],
  edges: []
})

const connection: SourceConnection = {
  id: 'conn-1',
  connectorId: 'github',
  name: 'owner/repo',
  filters: { owner: 'owner', repo: 'repo' },
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-08-04T00:00:00.000Z'
}

const item = (externalId: string): ConnectorItemContext => ({
  connectionId: connection.id,
  connectorId: connection.connectorId,
  externalId,
  title: `Item ${externalId}`,
  raw: { externalId, title: `Item ${externalId}` }
})

function record(workflowId: string, cursor: string, externalIds: string[]): number {
  return dbRecordConnectorPollPage({
    workflowId,
    connectionId: connection.id,
    connectorId: connection.connectorId,
    cursor,
    polledAt: '2026-08-04T01:00:00.000Z',
    events: externalIds.map((externalId) => ({
      eventId: externalId,
      eventType: 'issueCreated',
      eventTimestamp: '2026-08-04T00:30:00.000Z',
      connectorItem: item(externalId)
    }))
  })
}

beforeEach(() => {
  teardown = initTestDatabase()
  dbInsertSourceConnection(connection)
  dbInsertWorkflow(workflow('wf-issues'))
  dbInsertWorkflow(workflow('wf-prs'))
})

afterEach(() => teardown())

describe('durable connector inbox', () => {
  it('persists events and their cursor as one poll page', () => {
    expect(record('wf-issues', 'cursor-1', ['1', '2'])).toBe(2)
    expect(dbGetConnectorPollCursor('wf-issues', connection.id)).toBe('cursor-1')

    const claimed = dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-04T02:00:00.000Z',
      limit: 10
    })
    expect(claimed.map((entry) => entry.connectorItem.externalId)).toEqual(['1', '2'])
    expect(claimed.every((entry) => entry.attempts === 1)).toBe(true)
  })

  it('deduplicates an overlapping remote page without losing the checkpoint', () => {
    expect(record('wf-issues', 'cursor-1', ['1', '2'])).toBe(2)
    expect(record('wf-issues', 'cursor-2', ['2', '3'])).toBe(1)
    expect(dbGetConnectorPollCursor('wf-issues', connection.id)).toBe('cursor-2')

    const claimed = dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-04T02:00:00.000Z',
      limit: 10
    })
    expect(claimed.map((entry) => entry.connectorItem.externalId)).toEqual(['1', '2', '3'])
  })

  it('keeps independent cursors for workflows on one connection', () => {
    record('wf-issues', 'issue-cursor', [])
    record('wf-prs', 'pr-cursor', [])
    expect(dbGetConnectorPollCursor('wf-issues', connection.id)).toBe('issue-cursor')
    expect(dbGetConnectorPollCursor('wf-prs', connection.id)).toBe('pr-cursor')
  })

  it('resets the cursor and dedupe scope when a workflow changes connection', () => {
    record('wf-issues', 'cursor-1', ['1'])
    const secondConnection = { ...connection, id: 'conn-2', name: 'owner/other' }
    dbInsertSourceConnection(secondConnection)

    expect(dbGetConnectorPollCursor('wf-issues', secondConnection.id)).toBeUndefined()
    expect(
      dbRecordConnectorPollPage({
        workflowId: 'wf-issues',
        connectionId: secondConnection.id,
        connectorId: secondConnection.connectorId,
        cursor: 'cursor-2',
        polledAt: '2026-08-04T02:00:00.000Z',
        events: [
          {
            eventId: '1',
            eventType: 'issueCreated',
            eventTimestamp: '2026-08-04T01:30:00.000Z',
            connectorItem: { ...item('1'), connectionId: secondConnection.id }
          }
        ]
      })
    ).toBe(1)
  })

  it('does not reclaim a completed item', () => {
    record('wf-issues', 'cursor-1', ['1'])
    const [claimed] = dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-04T02:00:00.000Z',
      limit: 10
    })
    dbCompleteConnectorInbox(claimed.id, claimed.leaseToken, '2026-08-04T01:01:00.000Z')
    expect(
      dbClaimConnectorInbox({
        now: '2026-08-04T03:00:00.000Z',
        leaseUntil: '2026-08-04T04:00:00.000Z',
        limit: 10
      })
    ).toEqual([])
  })

  it('retries a failed item only after its backoff', () => {
    record('wf-issues', 'cursor-1', ['1'])
    const [claimed] = dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-04T02:00:00.000Z',
      limit: 10
    })
    dbRetryConnectorInbox({
      id: claimed.id,
      leaseToken: claimed.leaseToken,
      error: 'workflow failed',
      now: '2026-08-04T01:00:00.000Z'
    })

    expect(
      dbClaimConnectorInbox({
        now: '2026-08-04T01:00:59.000Z',
        leaseUntil: '2026-08-04T02:00:00.000Z',
        limit: 10
      })
    ).toEqual([])
    expect(
      dbClaimConnectorInbox({
        now: '2026-08-04T01:01:00.000Z',
        leaseUntil: '2026-08-04T02:00:00.000Z',
        limit: 10
      })[0].attempts
    ).toBe(2)
  })

  it('defers an unaccepted item without counting a workflow attempt', () => {
    record('wf-issues', 'cursor-1', ['1'])
    const [claimed] = dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-05T01:00:00.000Z',
      limit: 10
    })
    expect(claimed.attempts).toBe(1)
    dbDeferConnectorInbox(claimed.id, claimed.leaseToken, '2026-08-04T01:00:30.000Z')
    const [reclaimed] = dbClaimConnectorInbox({
      now: '2026-08-04T01:00:30.000Z',
      leaseUntil: '2026-08-05T01:00:00.000Z',
      limit: 10
    })
    expect(reclaimed.attempts).toBe(1)
  })

  it('releases leases immediately after a server restart', () => {
    record('wf-issues', 'cursor-1', ['1'])
    dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-04T02:00:00.000Z',
      limit: 10
    })
    dbReleaseConnectorInboxLeases('2026-08-04T01:01:00.000Z')
    expect(
      dbClaimConnectorInbox({
        now: '2026-08-04T01:01:00.000Z',
        leaseUntil: '2026-08-04T02:00:00.000Z',
        limit: 10
      })[0].attempts
    ).toBe(2)
  })

  it('ignores acknowledgements from an expired lease owner', () => {
    record('wf-issues', 'cursor-1', ['1'])
    const [first] = dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-04T01:01:00.000Z',
      limit: 10
    })
    const [second] = dbClaimConnectorInbox({
      now: '2026-08-04T01:01:00.000Z',
      leaseUntil: '2026-08-04T01:06:00.000Z',
      limit: 10
    })

    expect(second.leaseToken).not.toBe(first.leaseToken)
    expect(dbDeferConnectorInbox(first.id, first.leaseToken, '2026-08-04T01:01:30.000Z')).toBe(
      false
    )
    expect(dbCompleteConnectorInbox(second.id, second.leaseToken, '2026-08-04T01:02:00.000Z')).toBe(
      true
    )
  })

  it('preserves inbox rows and poll cursors when saving an existing workflow', () => {
    record('wf-issues', 'cursor-1', ['1'])
    const config = loadConfig()
    config.workflows = config.workflows?.map((entry) =>
      entry.id === 'wf-issues' ? { ...entry, name: 'Renamed' } : entry
    )
    saveConfig(config)

    expect(dbGetConnectorPollCursor('wf-issues', connection.id)).toBe('cursor-1')
    expect(
      dbClaimConnectorInbox({
        now: '2026-08-04T01:00:00.000Z',
        leaseUntil: '2026-08-04T01:05:00.000Z',
        limit: 10
      })
    ).toHaveLength(1)
  })

  it('renews only the lease its current owner holds', () => {
    record('wf-issues', 'cursor-1', ['1'])
    const [claimed] = dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-04T01:05:00.000Z',
      limit: 10
    })

    expect(
      dbRenewConnectorInboxLease(claimed.id, claimed.leaseToken, '2026-08-04T01:10:00.000Z')
    ).toBe(true)
    expect(dbRenewConnectorInboxLease(claimed.id, 'stale-token', '2026-08-04T01:20:00.000Z')).toBe(
      false
    )
    // The renewal held the row past the original expiry.
    expect(
      dbClaimConnectorInbox({
        now: '2026-08-04T01:06:00.000Z',
        leaseUntil: '2026-08-04T01:11:00.000Z',
        limit: 10
      })
    ).toEqual([])
  })

  it('counts only unexpired leases so delivery stays within capacity', () => {
    record('wf-issues', 'cursor-1', ['1', '2'])
    dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-04T01:05:00.000Z',
      limit: 10
    })

    expect(dbCountActiveConnectorInboxLeases('2026-08-04T01:01:00.000Z')).toBe(2)
    expect(dbCountActiveConnectorInboxLeases('2026-08-04T01:06:00.000Z')).toBe(0)
  })

  it('records a poll failure without disturbing the last good cursor', () => {
    record('wf-issues', 'cursor-1', ['1'])
    dbRecordConnectorPollError({
      workflowId: 'wf-issues',
      connectionId: connection.id,
      error: 'rate limited',
      polledAt: '2026-08-04T01:05:00.000Z'
    })

    expect(dbGetConnectorPollCursor('wf-issues', connection.id)).toBe('cursor-1')
    expect(dbListSourceConnections()[0].lastSyncError).toBe('rate limited')
  })

  it('finds the run that already owns an inbox row', () => {
    saveWorkflowRun({
      runId: 'run-inbox-9',
      workflowId: 'wf-issues',
      startedAt: '2026-08-04T01:00:00.000Z',
      status: 'running',
      connectorInboxId: 9,
      connectorInboxLeaseToken: 'lease-9',
      nodeStates: []
    })

    expect(dbGetWorkflowRunByConnectorInboxId(9)?.runId).toBe('run-inbox-9')
    expect(dbGetWorkflowRunByConnectorInboxId(404)).toBeNull()
  })
})

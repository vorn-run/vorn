import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type {
  ConnectorItemContext,
  SourceConnection,
  WorkflowDefinition
} from '../packages/shared/src/types'
import {
  dbClaimConnectorInbox,
  dbCompleteConnectorInbox,
  dbDeferConnectorInbox,
  dbGetConnectorPollCursor,
  dbInsertSourceConnection,
  dbInsertWorkflow,
  dbRecordConnectorPollPage,
  dbReleaseConnectorInboxLeases,
  dbRetryConnectorInbox,
  initTestDatabase
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
    expect(dbGetConnectorPollCursor('wf-issues')).toBe('cursor-1')

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
    expect(dbGetConnectorPollCursor('wf-issues')).toBe('cursor-2')

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
    expect(dbGetConnectorPollCursor('wf-issues')).toBe('issue-cursor')
    expect(dbGetConnectorPollCursor('wf-prs')).toBe('pr-cursor')
  })

  it('does not reclaim a completed item', () => {
    record('wf-issues', 'cursor-1', ['1'])
    const [claimed] = dbClaimConnectorInbox({
      now: '2026-08-04T01:00:00.000Z',
      leaseUntil: '2026-08-04T02:00:00.000Z',
      limit: 10
    })
    dbCompleteConnectorInbox(claimed.id, '2026-08-04T01:01:00.000Z')
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
    dbDeferConnectorInbox(claimed.id, '2026-08-04T01:00:30.000Z')
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
})

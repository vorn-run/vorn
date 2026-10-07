import type {
  ConnectorItemContext,
  ConnectorPollTriggerConfig,
  PollResult,
  SourceConnection,
  TriggerConfig
} from '@vornrun/shared/types'
import {
  dbGetConnectorPollCursor,
  dbGetSourceConnection,
  dbGetWorkflow,
  dbRecordConnectorPollError,
  dbRecordConnectorPollPage
} from './database'
import { connectorRegistry, applyDecryptedCreds } from './connectors'
import log from './logger'

const MAX_POLL_PAGES_PER_TICK = 20

/** How a connection is polled page by page, or why it cannot be. */
function connectionPager(
  conn: SourceConnection,
  event: string
): ((cursor?: string) => Promise<PollResult>) | string {
  const connector = connectorRegistry.get(conn.connectorId)
  if (connector?.pollConnection) return connector.pollConnection(conn, event)
  if (connector?.poll) {
    const found = connector
    return (cursor) => found.poll!(event, applyDecryptedCreds(conn), cursor)
  }
  return 'has no poll()'
}

function pollTrigger(workflowId: string): ConnectorPollTriggerConfig | null {
  const trigger = dbGetWorkflow(workflowId)?.nodes.find((n) => n.type === 'trigger')
  const config = (trigger?.config as TriggerConfig | undefined) ?? null
  return config?.triggerType === 'connectorPoll' ? config : null
}

/**
 * `connector:poll`: fetch a connector-poll workflow's new items into the inbox.
 *
 * vornd fires the schedule and runs the items; the connectors are this
 * server's, so fetching is done here. Each bounded remote page and its cursor
 * are committed to the inbox together; a connector failure records the error
 * and never advances past the last page safely kept.
 */
export async function pollConnector(workflowId: string): Promise<{ pages: number }> {
  const trigger = pollTrigger(workflowId)
  if (!trigger) return { pages: 0 }
  const conn = dbGetSourceConnection(trigger.connectionId)
  if (!conn) {
    log.warn(`[scheduler] connectorPoll: connection ${trigger.connectionId} not found — skipping`)
    return { pages: 0 }
  }
  const pager = connectionPager(conn, trigger.event)
  if (typeof pager === 'string') {
    log.warn(`[scheduler] connectorPoll: connection ${conn.id} ${pager} — skipping`)
    return { pages: 0 }
  }

  let cursor = dbGetConnectorPollCursor(workflowId, conn.id)
  const now = new Date().toISOString()
  let pages = 0
  try {
    for (let page = 0; page < MAX_POLL_PAGES_PER_TICK; page++) {
      const result = await pager(cursor)
      const nextCursor = result.nextCursor ?? cursor
      if (result.hasMore && nextCursor === cursor) {
        throw new Error(
          `${conn.connectorId}.poll(${trigger.event}) returned hasMore without advancing its cursor`
        )
      }
      const events = result.events.map((event) => {
        const data = event.data as Record<string, unknown>
        const connectorItem: ConnectorItemContext = {
          connectionId: conn.id,
          connectorId: conn.connectorId,
          externalId: String(data.externalId ?? event.id),
          externalUrl: typeof data.url === 'string' ? data.url : undefined,
          title: typeof data.title === 'string' ? data.title : String(data.title ?? ''),
          body: typeof data.description === 'string' ? data.description : undefined,
          raw: data
        }
        return {
          eventId: event.id,
          eventType: event.type,
          eventTimestamp: event.timestamp,
          connectorItem
        }
      })
      dbRecordConnectorPollPage({
        workflowId,
        connectionId: conn.id,
        connectorId: conn.connectorId,
        cursor: nextCursor,
        polledAt: now,
        events
      })
      pages++
      cursor = nextCursor
      if (!result.hasMore) break
    }
  } catch (err) {
    const errorMsg = err instanceof Error ? err.message : String(err)
    log.error(
      `[scheduler] connectorPoll: ${conn.connectorId}.poll(${trigger.event}) failed: ${errorMsg}`
    )
    dbRecordConnectorPollError({
      workflowId,
      connectionId: conn.id,
      error: errorMsg,
      polledAt: now
    })
  }
  return { pages }
}

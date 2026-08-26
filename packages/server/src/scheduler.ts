import cron, { type ScheduledTask } from 'node-cron'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { EventEmitter } from 'node:events'
import {
  WorkflowDefinition,
  TriggerConfig,
  ConnectorPollTriggerConfig,
  ConnectorItemContext,
  IPC
} from '@vornrun/shared/types'
import { configManager } from './config-manager'
import {
  dbClaimConnectorInbox,
  dbCompleteConnectorInbox,
  dbCountActiveConnectorInboxLeases,
  dbDeferConnectorInbox,
  dbGetConnectorPollCursor,
  dbGetSourceConnection,
  dbGetWorkflowRunByConnectorInboxId,
  dbRecordConnectorPollError,
  dbRecordConnectorPollPage,
  dbReleaseConnectorInboxLeases,
  dbRenewConnectorInboxLease,
  dbRetryConnectorInbox
} from './database'
import { connectorRegistry, applyDecryptedCreds } from './connectors'
import { MCP_CONNECTOR_ID, MCP_POLL_EVENT, pollMcpConnection } from './connectors/mcp'
import { clientRegistry } from './broadcast'
import log from './logger'

const LOCK_DIR = path.join(os.homedir(), '.vorn')
const INBOX_LEASE_MS = 5 * 60_000
const INBOX_DRAIN_INTERVAL_MS = 30_000
const INBOX_BATCH_SIZE = 50
const MAX_POLL_PAGES_PER_TICK = 20

/**
 * Try to acquire an execution lock for a workflow run.
 * Uses exclusive file creation (wx flag) keyed by the current minute
 * so it's atomic across processes and auto-expires for the next run.
 */
function acquireExecutionLock(workflowId: string): boolean {
  // Key by current minute so the lock naturally expires for the next scheduled run
  const minuteKey = Math.floor(Date.now() / 60_000)
  const lockFile = path.join(LOCK_DIR, `scheduler-${workflowId}-${minuteKey}.lock`)
  try {
    // wx flag: exclusive create — fails if file already exists (atomic)
    fs.writeFileSync(lockFile, String(process.pid), { flag: 'wx' })
    // Clean up stale lock files from previous runs
    cleanStaleLocks(workflowId, minuteKey)
    return true
  } catch {
    return false // Another instance already created this lock
  }
}

function cleanStaleLocks(workflowId: string, currentKey: number): void {
  try {
    const prefix = `scheduler-${workflowId}-`
    for (const f of fs.readdirSync(LOCK_DIR)) {
      if (f.startsWith(prefix) && f.endsWith('.lock')) {
        const key = parseInt(f.slice(prefix.length, -5), 10)
        if (!isNaN(key) && key < currentKey) {
          fs.unlinkSync(path.join(LOCK_DIR, f))
        }
      }
    }
  } catch {
    // Best-effort cleanup
  }
}

export interface MissedSchedule {
  workflow: WorkflowDefinition
  scheduledFor: string
}

function getTriggerConfig(wf: WorkflowDefinition): TriggerConfig | null {
  const triggerNode = wf.nodes.find((n) => n.type === 'trigger')
  if (!triggerNode) return null
  return triggerNode.config as TriggerConfig
}

class Scheduler extends EventEmitter {
  /**
   * Armed schedules that this server can act on with nobody attached.
   *
   * Only connector polls. `dispatchConnectorPoll` really does the work here --
   * it calls the connector and writes inbox rows without any client -- whereas a
   * recurring or one-off trigger only broadcasts `SCHEDULER_EXECUTE` for a
   * renderer to execute, so staying awake for one buys nothing: with no client
   * the occurrence is emitted, the minute lock is written, and the run is lost.
   * Holding the server open for that would be keeping a promise by dropping it.
   *
   * It matters which: connector polls are seeded enabled when a connector is
   * installed, so counting every armed cron meant anyone with a connector had a
   * server that never exited.
   */
  serverSideScheduleCount(): number {
    return this.connectorPollWorkflowIds.size
  }

  private cronJobs = new Map<string, ScheduledTask>()
  /** Of those, the ones that do real work here rather than in a renderer. */
  private connectorPollWorkflowIds = new Set<string>()
  private timeouts = new Map<string, NodeJS.Timeout>()
  private inboxTimer: NodeJS.Timeout | null = null

  startInboxWorker(): void {
    if (this.inboxTimer) return
    dbReleaseConnectorInboxLeases(new Date().toISOString())
    this.deliverPendingConnectorInbox()
    this.inboxTimer = setInterval(
      () => this.deliverPendingConnectorInbox(),
      INBOX_DRAIN_INTERVAL_MS
    )
  }

  completeConnectorInbox(
    id: number,
    leaseToken: string,
    disposition: 'processed' | 'retry' | 'defer',
    error?: string
  ): void {
    const now = new Date().toISOString()
    if (disposition === 'processed') {
      dbCompleteConnectorInbox(id, leaseToken, now)
    } else if (disposition === 'retry') {
      dbRetryConnectorInbox({
        id,
        leaseToken,
        error: error || 'Connector workflow failed',
        now
      })
    } else {
      dbDeferConnectorInbox(id, leaseToken, new Date(Date.now() + 30_000).toISOString())
    }
    this.deliverPendingConnectorInbox()
  }

  renewConnectorInbox(id: number, leaseToken: string): boolean {
    return dbRenewConnectorInboxLease(
      id,
      leaseToken,
      new Date(Date.now() + INBOX_LEASE_MS).toISOString()
    )
  }

  deliverPendingConnectorInbox(): void {
    // Claiming without a receiver would hide the row behind its lease until it
    // expires. Leave it pending and drain immediately when a client connects.
    if (clientRegistry.size === 0) return
    const now = Date.now()
    const nowIso = new Date(now).toISOString()
    const availableSlots = INBOX_BATCH_SIZE - dbCountActiveConnectorInboxLeases(nowIso)
    if (availableSlots <= 0) return
    const claimed = dbClaimConnectorInbox({
      now: nowIso,
      leaseUntil: new Date(now + INBOX_LEASE_MS).toISOString(),
      limit: availableSlots
    })
    for (const item of claimed) {
      const existingExecution = dbGetWorkflowRunByConnectorInboxId(item.id)
      if (
        existingExecution &&
        existingExecution.status !== 'running' &&
        (existingExecution.connectorInboxDisposition === 'processed' ||
          existingExecution.status === 'success' ||
          existingExecution.status === 'cancelled')
      ) {
        dbCompleteConnectorInbox(item.id, item.leaseToken, new Date().toISOString())
        continue
      }
      this.emit('client-message', IPC.SCHEDULER_EXECUTE, {
        workflowId: item.workflowId,
        connectorItem: {
          ...item.connectorItem,
          inboxId: item.id,
          inboxLeaseToken: item.leaseToken
        },
        connectorInboxId: item.id,
        connectorInboxLeaseToken: item.leaseToken,
        ...(existingExecution?.status === 'running' && { existingExecution })
      })
    }
  }

  syncSchedules(workflows: WorkflowDefinition[]): void {
    log.info(
      `[scheduler] syncing ${workflows.length} workflows (active crons: ${this.cronJobs.size}, timeouts: ${this.timeouts.size})`
    )

    // Cancel jobs for workflows that no longer exist or are disabled
    for (const [id] of this.cronJobs) {
      const wf = workflows.find((w) => w.id === id)
      const trigger = wf ? getTriggerConfig(wf) : null
      const kind = trigger?.triggerType
      if (!wf || !wf.enabled || (kind !== 'recurring' && kind !== 'connectorPoll')) {
        this.cronJobs.get(id)?.stop()
        this.cronJobs.delete(id)
        this.connectorPollWorkflowIds.delete(id)
      }
    }
    for (const [id] of this.timeouts) {
      const wf = workflows.find((w) => w.id === id)
      const trigger = wf ? getTriggerConfig(wf) : null
      if (!wf || !wf.enabled || trigger?.triggerType !== 'once') {
        clearTimeout(this.timeouts.get(id)!)
        this.timeouts.delete(id)
      }
    }

    // Register new/updated schedules
    for (const wf of workflows) {
      if (!wf.enabled) {
        log.info(`[scheduler] skipping disabled workflow "${wf.name}"`)
        continue
      }
      const trigger = getTriggerConfig(wf)
      if (!trigger) {
        log.info(`[scheduler] no trigger node for workflow "${wf.name}"`)
        continue
      }
      log.info(`[scheduler] workflow "${wf.name}" trigger=${trigger.triggerType}`)

      if (
        (trigger.triggerType === 'recurring' || trigger.triggerType === 'connectorPoll') &&
        !this.cronJobs.has(wf.id)
      ) {
        log.info(
          `[scheduler] registering ${trigger.triggerType} workflow "${wf.name}" cron="${trigger.cron}" enabled=${wf.enabled}`
        )
        if (!cron.validate(trigger.cron)) {
          log.error(
            `[scheduler] invalid cron expression for workflow "${wf.name}": ${trigger.cron}`
          )
          continue
        }
        try {
          const task = cron.schedule(trigger.cron, () => this.executeWorkflow(wf.id), {
            timezone: trigger.timezone || Intl.DateTimeFormat().resolvedOptions().timeZone
          })
          this.cronJobs.set(wf.id, task)
          if (trigger.triggerType === 'connectorPoll') this.connectorPollWorkflowIds.add(wf.id)
        } catch (err) {
          log.error({ err }, `[scheduler] failed to schedule workflow "${wf.name}":`)
        }
      }

      // Membership is derived from the trigger as it stands now, not from what
      // it was when the job was created. Both loops above keep an existing cron
      // job when the new kind is still cron-eligible, and the registration below
      // is skipped for an id that already has one -- so an edit from `recurring`
      // to `connectorPoll` would never add the id, and the reverse edit would
      // leave it behind. Either way the count that decides whether this server
      // may leave stops describing the schedules it actually holds.
      if (this.cronJobs.has(wf.id)) {
        if (trigger.triggerType === 'connectorPoll') this.connectorPollWorkflowIds.add(wf.id)
        else this.connectorPollWorkflowIds.delete(wf.id)
      }

      if (trigger.triggerType === 'once' && !this.timeouts.has(wf.id)) {
        const runAt = new Date(trigger.runAt).getTime()
        if (isNaN(runAt)) {
          log.error(`[scheduler] invalid runAt date for workflow "${wf.name}": ${trigger.runAt}`)
          continue
        }
        const delay = runAt - Date.now()
        if (delay > 0) {
          // Cap delay to 24 hours to avoid setTimeout overflow (max ~24.8 days)
          // The scheduler will re-evaluate on next syncSchedules call
          const MAX_DELAY = 24 * 60 * 60 * 1000
          const safeDelay = Math.min(delay, MAX_DELAY)
          const timer = setTimeout(() => {
            if (safeDelay < delay) {
              // Re-schedule: not yet time to fire
              this.timeouts.delete(wf.id)
              this.syncSchedules(configManager.loadConfig().workflows ?? [])
            } else {
              this.executeWorkflow(wf.id)
            }
          }, safeDelay)
          this.timeouts.set(wf.id, timer)
        }
      }
    }
  }

  private executeWorkflow(workflowId: string, inputs?: Record<string, unknown>): void {
    if (!acquireExecutionLock(workflowId)) {
      log.info(`[scheduler] skipping workflow ${workflowId} — already executed by another instance`)
      this.timeouts.delete(workflowId)
      return
    }

    // Look up the workflow to decide whether this is a connector-poll fan-out
    // or a normal single-execution fire.
    const workflows = configManager.loadConfig().workflows ?? []
    const wf = workflows.find((w) => w.id === workflowId)
    if (!wf) {
      this.timeouts.delete(workflowId)
      return
    }
    const trigger = getTriggerConfig(wf)

    if (trigger?.triggerType === 'connectorPoll') {
      // Fire-and-forget — the dispatcher emits N SCHEDULER_EXECUTE events, one
      // per new item. Cursor advance and error recording happen inside.
      this.dispatchConnectorPoll(workflowId, trigger).catch((err) => {
        log.error(`[scheduler] connectorPoll dispatch failed for ${workflowId}:`, err)
      })
      this.timeouts.delete(workflowId)
      return
    }

    log.info(`[scheduler] executing workflow ${workflowId}`)
    this.emit('client-message', IPC.SCHEDULER_EXECUTE, { workflowId, inputs })
    this.timeouts.delete(workflowId)
  }

  /**
   * Poll a connector and fan out one workflow execution per new item.
   *
   * - Cursor lives per workflow, so subscriptions sharing a connection cannot
   *   skip each other's events.
   * - Each bounded remote page and its cursor are committed atomically to the
   *   durable inbox before anything is broadcast.
   * - Workflow failures release their lease with backoff; process crashes
   *   leave the row reclaimable.
   * - Connector-level failures record lastSyncError and do not advance beyond
   *   the last page that was safely persisted.
   */
  private async dispatchConnectorPoll(
    workflowId: string,
    trigger: ConnectorPollTriggerConfig
  ): Promise<void> {
    const conn = dbGetSourceConnection(trigger.connectionId)
    if (!conn) {
      log.warn(`[scheduler] connectorPoll: connection ${trigger.connectionId} not found — skipping`)
      return
    }
    const connector = connectorRegistry.get(conn.connectorId)
    // MCP is polymorphic: its poll needs the full SourceConnection to spawn the
    // per-connection stdio client, so it's routed through pollMcpConnection
    // rather than the generic connector.poll (which only gets flattened
    // filters) — mirroring how MCP execute is special-cased. Decrypted secrets
    // don't need overlaying here: getOrStartClient pulls secretEnv from the
    // decrypted-creds store keyed by conn.id.
    const isMcp = conn.connectorId === MCP_CONNECTOR_ID
    if (!isMcp && !connector?.poll) {
      log.warn(`[scheduler] connectorPoll: connector ${conn.connectorId} has no poll() — skipping`)
      return
    }
    // MCP defines exactly one event; reject a misconfigured trigger rather than
    // fan out on an event the connector never emits.
    if (isMcp && trigger.event !== MCP_POLL_EVENT) {
      log.warn(
        `[scheduler] connectorPoll: MCP connection ${conn.id} got unexpected event "${trigger.event}" — skipping`
      )
      return
    }

    let cursor = dbGetConnectorPollCursor(workflowId, conn.id)
    const now = new Date().toISOString()
    try {
      for (let page = 0; page < MAX_POLL_PAGES_PER_TICK; page++) {
        const result = isMcp
          ? await pollMcpConnection(conn, cursor)
          : await connector!.poll!(trigger.event, applyDecryptedCreds(conn), cursor)
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
      return
    }

    this.deliverPendingConnectorInbox()
  }

  checkMissedSchedules(workflows: WorkflowDefinition[]): MissedSchedule[] {
    const missed: MissedSchedule[] = []
    for (const wf of workflows) {
      if (!wf.enabled) continue
      const trigger = getTriggerConfig(wf)
      if (trigger?.triggerType === 'once') {
        const runAt = new Date(trigger.runAt).getTime()
        if (runAt < Date.now() && !wf.lastRunAt) {
          missed.push({ workflow: wf, scheduledFor: trigger.runAt })
        }
      }
    }
    return missed
  }

  getNextRun(workflowId: string, workflows: WorkflowDefinition[]): string | null {
    const wf = workflows.find((w) => w.id === workflowId)
    if (!wf || !wf.enabled) return null
    const trigger = getTriggerConfig(wf)
    if (!trigger) return null

    if (trigger.triggerType === 'once') {
      const runAt = new Date(trigger.runAt).getTime()
      return runAt > Date.now() ? trigger.runAt : null
    }

    if (trigger.triggerType === 'recurring' || trigger.triggerType === 'connectorPoll') {
      return trigger.cron
    }

    return null
  }

  /**
   * Trigger a workflow manually, bypassing the cron tick. Used by "Run now"
   * in settings for connector-seeded workflows: the same dispatch path as
   * cron, so no hidden logic — just a forced tick. The minute-key lock still
   * applies so repeated clicks within the same minute fold into one run.
   *
   * `inputs` carries the values a manual run was started with, forwarded to
   * the renderer so `{{inputs.*}}` resolves the same as a direct run.
   */
  triggerWorkflow(workflowId: string, inputs?: Record<string, unknown>): void {
    this.executeWorkflow(workflowId, inputs)
  }

  /**
   * Ask the instance running `runId` to stop it.
   *
   * No lock and no lookup: unlike a trigger, this must not be claimed by one
   * instance, because the one that answers may not be the one holding the run.
   * Every instance gets the message and only the owner acts on it.
   */
  stopRun(runId: string): void {
    log.info(`[scheduler] broadcasting stop for run ${runId}`)
    this.emit('client-message', IPC.SCHEDULER_STOP_RUN, { runId })
  }

  stopAll(): void {
    for (const [, job] of this.cronJobs) job.stop()
    for (const [, timer] of this.timeouts) clearTimeout(timer)
    this.cronJobs.clear()
    this.connectorPollWorkflowIds.clear()
    this.timeouts.clear()
    if (this.inboxTimer) clearInterval(this.inboxTimer)
    this.inboxTimer = null
  }
}

export const scheduler = new Scheduler()

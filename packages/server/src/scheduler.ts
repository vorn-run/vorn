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
import { dbGetSourceConnection, dbUpdateSourceConnection } from './database'
import { connectorRegistry, applyDecryptedCreds } from './connectors'
import { MCP_CONNECTOR_ID, pollMcpConnection } from './connectors/mcp'
import log from './logger'

const LOCK_DIR = path.join(os.homedir(), '.vorn')

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
  private cronJobs = new Map<string, ScheduledTask>()
  private timeouts = new Map<string, NodeJS.Timeout>()

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
        } catch (err) {
          log.error(`[scheduler] failed to schedule workflow "${wf.name}":`, err)
        }
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

  private executeWorkflow(workflowId: string): void {
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
    this.emit('client-message', IPC.SCHEDULER_EXECUTE, { workflowId })
    this.timeouts.delete(workflowId)
  }

  /**
   * Poll a connector and fan out one workflow execution per new item.
   *
   * - Cursor lives on the connection row; advanced after a successful poll.
   * - Per-item workflow failures do NOT stall the pipe — the cursor has
   *   already advanced past them; the failure shows up in run history and in
   *   the connection's lastSyncError field.
   * - Connector-level failures (poll() throws) record lastSyncError, emit one
   *   failed execution so run history surfaces it, and do not advance cursor.
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
    // filters) — mirroring how MCP execute is special-cased.
    const isMcp = conn.connectorId === MCP_CONNECTOR_ID
    if (!isMcp && !connector?.poll) {
      log.warn(`[scheduler] connectorPoll: connector ${conn.connectorId} has no poll() — skipping`)
      return
    }

    const cursor = conn.syncCursor
    const now = new Date().toISOString()
    let result
    try {
      result = isMcp
        ? await pollMcpConnection(conn, cursor)
        : await connector!.poll!(trigger.event, applyDecryptedCreds(conn), cursor)
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : String(err)
      log.error(
        `[scheduler] connectorPoll: ${conn.connectorId}.poll(${trigger.event}) failed: ${errorMsg}`
      )
      dbUpdateSourceConnection(conn.id, { lastSyncAt: now, lastSyncError: errorMsg })
      // Do not emit SCHEDULER_EXECUTE here. An event without a connectorItem
      // would bounce back through the renderer's connectorPoll guard (which
      // reroutes context-less runs via workflow:runManual), creating churn.
      // The failure is already visible via the connection's lastSyncError.
      return
    }

    // Advance cursor + clear last error atomically.
    dbUpdateSourceConnection(conn.id, {
      lastSyncAt: now,
      lastSyncError: undefined,
      syncCursor: result.nextCursor ?? cursor
    })

    if (result.events.length === 0) {
      log.info(`[scheduler] connectorPoll: no new items for ${conn.connectorId}:${trigger.event}`)
      return
    }

    log.info(
      `[scheduler] connectorPoll: fanning out ${result.events.length} item(s) for ${conn.connectorId}:${trigger.event}`
    )
    for (const event of result.events) {
      // Event data from the connector is the upstream item payload. The
      // GitHub connector puts an ExternalItem-shaped object into `data`.
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
      this.emit('client-message', IPC.SCHEDULER_EXECUTE, { workflowId, connectorItem })
    }
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
   */
  triggerWorkflow(workflowId: string): void {
    this.executeWorkflow(workflowId)
  }

  stopAll(): void {
    for (const [, job] of this.cronJobs) job.stop()
    for (const [, timer] of this.timeouts) clearTimeout(timer)
    this.cronJobs.clear()
    this.timeouts.clear()
  }
}

export const scheduler = new Scheduler()

import fs from 'fs'
import crypto from 'node:crypto'
import { registerMethod, registerNotification } from './ws-handler'
import { ptyManager } from './pty-manager'
import { headlessManager } from './headless-manager'
import { configManager } from './config-manager'
import { sessionManager } from './session-persistence'
import { scheduler } from './scheduler'
import { claimWorkflowRun, releaseWorkflowRun, type RunClaimRequest } from './workflow-run-claims'
import { scheduleLogManager } from './schedule-log'
import { getRecentSessions } from './agent-history'
import { detectIDEs, openInIDE } from './ide-detector'
import { detectMobileProject } from './mobile-detector'
import { detectInstalledAgents, clearAgentDetectionCache } from './agent-detector'
import { clientRegistry } from './broadcast'
import {
  restoredRecords,
  listRestored,
  consumeRestored,
  consumeAllRestored,
  restoreHeld
} from './restored-sessions'
import { clearScreen } from './terminal-screen'
import { discardHistory } from './history/writer'
import { buildRestorePayload } from '@vornrun/shared/session-restore'
import { clearScrollback, readScrollback } from './terminal-scrollback'
import {
  claimTranscriptFor,
  sessionToBindOnCreate,
  transcriptHolder,
  transcriptNamedOnCreate
} from './agent-transcript'
import {
  claimSpawningTranscript,
  releaseSpawningTranscript,
  releaseSpawningTranscriptsFor
} from './transcript-claims'
import { browserBridge } from './browser-bridge'
import { hookServer } from './hook-server'
import { hookStatusMapper } from './hook-status-mapper'
import { installHooks } from './hook-installer'
import {
  installCopilotHooks,
  uninstallCopilotHooks,
  CopilotHookInstallation
} from './copilot-hook-installer'
import {
  IPC,
  WidgetAgentInfo,
  PermissionRequestInfo,
  SessionEventType,
  RemoteHost,
  getProjectRemoteHostId,
  isTerminalTaskStatus
} from '@vornrun/shared/types'
import type {
  SourceConnection,
  TaskConfig,
  TaskStatus,
  ConnectorConfigField,
  ConnectorManifest,
  ExternalItem,
  ProjectConfig,
  WorktreeRetentionConfig
} from '@vornrun/shared/types'
import { connectionConnectorId, DEFAULT_ARTIFACT_DIRS } from '@vornrun/shared/types'
import * as gitUtils from './git-utils'
import { detectRepoSlug } from './git-utils'
import {
  scanWorktreeInventory,
  reclaimArtifacts,
  removeWorktrees,
  pruneOrphanDirs,
  deleteStaleBranches,
  measureWorktree,
  invalidateSizeCache
} from './worktree-inventory'
import { fileStamp, listDir, readFileContent, writeFileContent } from './file-utils'
import { listShellExecutables } from './shell-integration'
import { listInstalledShells } from './shell-integration/installed'
import {
  saveTaskImage,
  saveTaskImageFromBase64,
  deleteTaskImage,
  getTaskImagePath,
  cleanupTaskImages
} from './task-images'
import {
  saveWorkflowRun,
  listWorkflowRuns,
  listWorkflowRunsByTask,
  listAllWorkflowRuns,
  listRunsWithWaitingGates,
  listRunningRuns,
  updateWorkflowRunStatus,
  dbSaveSSHKey,
  dbListSSHKeys,
  dbGetSSHKey,
  dbDeleteSSHKey,
  insertSessionEvent,
  listSessionEvents,
  listSessionEventsBySession,
  dbListSourceConnections,
  dbGetSourceConnection,
  dbInsertSourceConnection,
  dbUpdateSourceConnection,
  dbDeleteSourceConnection,
  dbGetTaskSourceLink,
  dbGetTaskSourceLinkByExternalId,
  dbFindTaskByConnectorExternalId,
  dbInsertTaskSourceLink,
  dbUpdateTaskSourceLink,
  dbInsertTask,
  dbUpdateTask,
  dbDeleteTask,
  dbGetMaxTaskOrder,
  dbGetProject,
  dbSignalChange,
  dbInsertWorkflow,
  dbDeleteWorkflow,
  dbGetWorkflow,
  dbListWorkflows,
  dbUpdateWorkflow,
  dbListTasks,
  dbGetTask
} from './database'
import {
  connectorRegistry,
  setDecryptedCreds,
  clearDecryptedCreds,
  applyDecryptedCreds,
  invokeMcpTool,
  discoverTools,
  mcpConnectionActions,
  visibleMcpTools,
  stopMcpClient,
  stopClientsForConnector,
  connectionIdsForConnector,
  connectionsForConnector,
  inspectPack,
  installPack,
  removePack,
  rollbackPack,
  listInstalledPacks
} from './connectors'
import {
  MCP_CONNECTOR_ID,
  MCP_POLL_EVENT,
  backfillMcpConnection,
  preflightMcpConnection
} from './connectors/mcp'
import {
  httpConnector,
  httpProfileError,
  lockedProfileError,
  performHttpRequest
} from './connectors/http'
import { getDecryptedCreds } from './connectors/decrypted-creds'
import { listKeys, passwordFields } from './connectors/keys'
import { installedPack } from './connectors/packs'
import {
  syncImplicitConnection,
  type ImplicitConnectionDeps
} from './connectors/implicit-connection'
import { GH_SOURCE } from './connectors/gh-cli'
import { probeAuth, type BorrowSource } from './connectors/auth-rung'
import { resolveConnectorAuth } from './connectors/connector-auth'
import { probeSdkConnector, type SdkProbeRequest } from './connectors/sdk-probe'
import { isImplicitConnection, type ConnectorPackSource } from '@vornrun/shared/types'
import { catalogSnapshot, refreshCatalog } from './connectors/catalog'
import { forEachConnectorItem } from './connectors/paging'
import { buildConnectorSeededWorkflow } from './default-workflows'
import { connectorSeededWorkflowId, connectorSeededWorkflowIdPrefix } from '@vornrun/shared/types'
import { executeScript, scriptRunnerEvents } from './script-runner'
import { getTailscaleStatus, clearBinaryCache } from './tailscale'
import { reachableUrls } from './reachable-urls'
import { listTokens, mintOwnerToken, revokeToken } from './token-manager'
import {
  approveRequest,
  cancelPairing,
  denyRequest,
  pendingRequests,
  startPairing
} from './pairing'
import { disconnectToken } from './ws-handler'
import { testSshConnection } from './process-utils'
import { captureAgentSessionId } from './agent-session-capture'
import { supportsExactSessionResume, supportsSessionIdPinning } from '@vornrun/shared/types'
import log from './logger'

const copilotInstallations = new Map<string, CopilotHookInstallation>()

/**
 * What a status change does to the two dates that hang off it.
 *
 * Finishing a task stamps `completedAt`; reopening one clears it, and clears
 * `archivedAt` with it — a task that is live again cannot still be filed away.
 *
 * Both keys are returned rather than omitted, because `dbUpdateTask` decides
 * what to write with `'completedAt' in updates`: an absent key leaves the
 * column alone, and only an explicit `undefined` clears it.
 */
function terminalStamps(
  from: TaskStatus,
  to: TaskStatus
): { completedAt?: string; archivedAt?: string } {
  const was = isTerminalTaskStatus(from)
  const is = isTerminalTaskStatus(to)
  if (is && !was) return { completedAt: new Date().toISOString() }
  if (!is && was) return { completedAt: undefined, archivedAt: undefined }
  return {}
}

/**
 * Why a pack source cannot be used, or empty when it can.
 *
 * Stated at the boundary because this arrives from a renderer, the web shim or
 * an agent: an unknown shape would otherwise reach the installer, and a plain
 * `http` URL would fetch a connector's code over a link anyone can rewrite.
 */
function unusableSource(source: ConnectorPackSource): string {
  if (!source || typeof source !== 'object') return 'That is not a pack to install'
  switch (source.kind) {
    case 'file':
      return typeof source.path === 'string' && source.path !== '' ? '' : 'That file path is empty'
    case 'npm':
      return typeof source.packageName === 'string' && source.packageName !== ''
        ? ''
        : 'That package name is empty'
    case 'staged':
      return typeof source.token === 'string' && source.token !== '' ? '' : 'That pack has expired'
    case 'url': {
      let url: URL
      try {
        url = new URL(source.url)
      } catch {
        return 'That is not a URL a pack can be fetched from'
      }
      if (url.protocol === 'https:') return ''
      const local = url.hostname === 'localhost' || url.hostname === '127.0.0.1'
      return url.protocol === 'http:' && local
        ? ''
        : 'A pack is fetched over https, or from this machine'
    }
    default:
      return 'That is not a pack to install'
  }
}

// The built-in GitHub connector predates rungs and declares its auth beside its client.
async function authForConnector(connectorId: string): Promise<BorrowSource> {
  if (connectorId === 'github') return GH_SOURCE
  return (await resolveConnectorAuth(connectorId)) ?? { auth: undefined, declared: [] }
}

// Make a connection and everything that comes with one.
function createConnectionRecord(
  params: Omit<
    SourceConnection,
    'id' | 'createdAt' | 'lastSyncAt' | 'lastSyncError' | 'syncCursor'
  > & {
    seedWorkflow?: { name: string; defaultCronFromMinutes: number }
  },
  options: { discover?: boolean } = {}
): SourceConnection {
  const id = crypto.randomUUID()
  const conn: SourceConnection = {
    id,
    connectorId: params.connectorId,
    name: params.name,
    filters: params.filters,
    syncIntervalMinutes: params.syncIntervalMinutes,
    statusMapping: params.statusMapping,
    ...(params.executionProject && { executionProject: params.executionProject }),
    createdAt: new Date().toISOString()
  }
  dbInsertSourceConnection(conn)

  // Seed visible + editable default workflows.
  const connector = connectorRegistry.get(conn.connectorId)
  const manifest = connector?.describe()
  const seeded: Array<NonNullable<ConnectorManifest['defaultWorkflows']>[number]> = [
    ...(manifest?.defaultWorkflows ?? []),
    // Only for MCP: no other connector emits MCP_POLL_EVENT.
    ...(params.seedWorkflow && params.connectorId === MCP_CONNECTOR_ID
      ? [
          {
            name: params.seedWorkflow.name,
            event: MCP_POLL_EVENT,
            defaultCronFromMinutes: params.seedWorkflow.defaultCronFromMinutes,
            downstream: 'createTaskFromItem' as const
          }
        ]
      : [])
  ]
  if (manifest) {
    for (const event of seeded) {
      const wfId = connectorSeededWorkflowId(conn.id, event.event)
      if (dbGetWorkflow(wfId)) continue
      // The connection's own mapping beats the connector's suggestion: it is what the person setting it up actually chose.
      const withMapping: ConnectorManifest = {
        ...manifest,
        statusMapping: Object.entries(conn.statusMapping ?? {}).map(([upstream, local]) => ({
          upstream,
          suggestedLocal: local
        }))
      }
      const wf = buildConnectorSeededWorkflow(conn, withMapping, event)
      dbInsertWorkflow(wf)
      log.info(`[connector] seeded workflow ${wfId} for connection ${conn.id}`)
    }
  }

  dbSignalChange()
  configManager.notifyChanged()

  // For MCP connections, kick off tool discovery in the background.
  if (conn.connectorId === MCP_CONNECTOR_ID && options.discover !== false) {
    setTimeout(() => {
      void runMcpDiscovery(conn.id).catch((err) =>
        log.warn(`[mcp] initial discovery failed for ${conn.id}: ${err}`)
      )
    }, 1500)
  }

  return conn
}

// Take a connection out, with everything that only existed for it.
function deleteConnectionRecord(id: string): void {
  // Delete any seeded workflows tied to this connection.
  const prefix = connectorSeededWorkflowIdPrefix(id)
  for (const wf of dbListWorkflows()) {
    if (wf.id.startsWith(prefix)) {
      dbDeleteWorkflow(wf.id)
    }
  }
  // task_source_links cascade via FK.
  dbDeleteSourceConnection(id)
  // Forget any decrypted plaintext for this connection.
  clearDecryptedCreds(id)
  // Terminate any live MCP stdio child for this connection.
  void stopMcpClient(id).catch((err) => log.warn(`[mcp] stopClient failed: ${err}`))
  dbSignalChange()
  configManager.notifyChanged()
}

/** The edges the implicit-connection rule acts through, wired to this server. */
const implicitConnectionDeps: ImplicitConnectionDeps = {
  list: () => dbListSourceConnections(),
  // The caller runs discovery once the connection exists.
  create: (params) => createConnectionRecord({ ...params, statusMapping: {} }, { discover: false }),
  remove: (connectionId) => {
    deleteConnectionRecord(connectionId)
    log.info(`[packs] withdrew the implicit connection ${connectionId}`)
  },
  changed: () => {
    dbSignalChange()
    configManager.notifyChanged()
  }
}

// The install-time rule, applied once at startup to packs installed before it existed.
export function reconcileImplicitConnections(): void {
  try {
    for (const pack of listInstalledPacks()) {
      const made = syncImplicitConnection(pack.id, pack, implicitConnectionDeps, MCP_CONNECTOR_ID)
      if (!made) continue
      void runMcpDiscovery(made.id).catch((err) =>
        log.warn(`[packs] discovery failed for ${made.id}: ${err}`)
      )
    }
  } catch (err) {
    log.warn(`[packs] could not reconcile implicit connections: ${err}`)
  }
}

/**
 * Settle every connection of a connector whose files just changed.
 *
 * Stopping the child is only half of it: the tools a step can call were
 * discovered from the version that is now gone, so a rename or a removed
 * action would keep being offered until someone refreshed the row by hand.
 */
async function onPackChanged(connectorId: string): Promise<void> {
  await stopClientsForConnector(connectorId)
  // Before the ids are read, so a connector that just connected itself is discovered too.
  syncImplicitConnection(
    connectorId,
    installedPack(connectorId),
    implicitConnectionDeps,
    MCP_CONNECTOR_ID
  )
  await Promise.allSettled(
    connectionIdsForConnector(connectorId).map((id) =>
      runMcpDiscovery(id).catch((err) => log.warn(`[packs] rediscovery failed for ${id}: ${err}`))
    )
  )
}

/** Discover tools on an MCP connection and persist them on the row. */
async function runMcpDiscovery(
  connectionId: string
): Promise<{ ok: boolean; count?: number; error?: string }> {
  const conn = dbGetSourceConnection(connectionId)
  if (!conn || conn.connectorId !== MCP_CONNECTOR_ID) {
    return { ok: false, error: 'Not an MCP connection' }
  }
  try {
    const tools = await discoverTools(conn)
    dbUpdateSourceConnection(conn.id, {
      filters: { ...conn.filters, discoveredTools: tools },
      lastSyncAt: new Date().toISOString(),
      lastSyncError: undefined
    })
    dbSignalChange()
    configManager.notifyChanged()
    return { ok: true, count: tools.length }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    dbUpdateSourceConnection(conn.id, { lastSyncError: msg })
    dbSignalChange()
    configManager.notifyChanged()
    return { ok: false, error: msg }
  }
}

/**
 * Upsert an external connector item into the task board. Three-tier dedup:
 *   1. Link exists (same conn + external id) → update task fields + link.
 *   2. Orphan task exists (prior link cascade-deleted) → re-adopt under
 *      the current connection so we don't make duplicates.
 *   3. Neither → create a fresh task + link.
 * Shared by `connection:upsertFromItem` (per-item workflow fan-out) and
 * `connection:backfill` (manual bulk import). Caller handles the
 * `lastSyncAt` bump and notify/signal plumbing.
 */
function upsertExternalItem(
  conn: SourceConnection,
  item: {
    externalId: string
    title: string
    description: string
    externalUrl: string
    sourceStatusRaw: string
    sourceUpdatedAt: string
  },
  opts: { projectName: string; initialStatus: TaskStatus; now: string }
): { taskId: string; created: boolean } {
  const { projectName, initialStatus, now } = opts

  const existing = dbGetTaskSourceLinkByExternalId(conn.id, item.externalId)
  if (existing) {
    dbUpdateTask(existing.taskId, {
      title: item.title,
      description: item.description,
      updatedAt: now,
      sourceExternalUrl: item.externalUrl,
      sourceExternalId: item.externalId
    })
    dbUpdateTaskSourceLink(existing.taskId, {
      sourceStatusRaw: item.sourceStatusRaw,
      sourceUpdatedAt: item.sourceUpdatedAt,
      lastSyncedAt: now
    })
    return { taskId: existing.taskId, created: false }
  }

  // The same id the task was written under, or a re-added packaged connection
  // adopts nothing and its items arrive a second time.
  const orphan = dbFindTaskByConnectorExternalId(connectionConnectorId(conn), item.externalId)
  if (orphan) {
    dbUpdateTask(orphan.id, {
      title: item.title,
      description: item.description,
      updatedAt: now,
      sourceExternalUrl: item.externalUrl,
      sourceExternalId: item.externalId
    })
    dbInsertTaskSourceLink({
      taskId: orphan.id,
      connectionId: conn.id,
      connectorId: conn.connectorId,
      externalId: item.externalId,
      externalUrl: item.externalUrl,
      sourceStatusRaw: item.sourceStatusRaw,
      sourceUpdatedAt: item.sourceUpdatedAt,
      lastSyncedAt: now,
      conflictState: 'none'
    })
    log.info(
      `[upsertExternalItem] re-adopted orphan task ${orphan.id} for ${conn.connectorId}:${item.externalId}`
    )
    return { taskId: orphan.id, created: false }
  }

  const taskId = crypto.randomUUID()
  const maxOrder = dbGetMaxTaskOrder(projectName)
  dbInsertTask({
    id: taskId,
    projectName,
    title: item.title,
    description: item.description,
    status: initialStatus,
    order: maxOrder + 1,
    createdAt: now,
    updatedAt: now,
    // The connector's own id, not the `mcp` every packaged one is stored under.
    sourceConnectorId: connectionConnectorId(conn),
    sourceExternalId: item.externalId,
    ...(item.externalUrl && { sourceExternalUrl: item.externalUrl })
  })
  dbInsertTaskSourceLink({
    taskId,
    connectionId: conn.id,
    connectorId: conn.connectorId,
    externalId: item.externalId,
    externalUrl: item.externalUrl,
    sourceStatusRaw: item.sourceStatusRaw,
    sourceUpdatedAt: item.sourceUpdatedAt,
    lastSyncedAt: now,
    conflictState: 'none'
  })
  return { taskId, created: true }
}

function logSessionEvent(
  sessionId: string,
  eventType: SessionEventType,
  metadata?: Record<string, unknown>
): void {
  try {
    insertSessionEvent({
      sessionId,
      eventType,
      timestamp: new Date().toISOString(),
      ...(metadata ? { metadata } : {})
    })
  } catch (err) {
    log.error({ err }, '[session-events] failed to log event:')
  }
}

let serverPort = 0
export function setServerPort(port: number): void {
  serverPort = port
}

/**
 * Let go of everything held for a session from a previous run.
 *
 * Called when one is claimed, closed or dismissed -- the three ways a carried
 * over record stops being offered. All three end the same way, so they end in
 * one place.
 *
 * At module scope rather than inside `registerAllMethods` because it captures
 * nothing from it, and out here it can be tested without standing up a socket.
 */
/** Whether a path is a directory right now, answering false for every other case. */
function isDirectory(at: string): boolean {
  try {
    return fs.statSync(at).isDirectory()
  } catch {
    return false
  }
}

export async function forgetRestored(id: string): Promise<void> {
  clearScreen(id)
  // Recovery seeds a restored session's scrollback so a pane can be shown one
  // (`history/recovery.ts`). Nothing else ever frees it: this session has no PTY,
  // so it never reaches the `clearScrollback` on the kill path, and letting the
  // record go is the last thing that happens to it. Without this the bytes stay
  // held for the life of the server -- the same reasoning that put `clearScreen`
  // here.
  clearScrollback(id)
  await discardHistory(id)
}

export function registerAllMethods(): void {
  // Wire headless worktree counter into pty-manager for cleanup gating
  ptyManager.setHeadlessWorktreeCounter((worktreePath, excludeId) =>
    headlessManager.getActiveSessionsForWorktree(worktreePath, excludeId)
  )

  // Terminal
  registerMethod('terminal:create', (payload) => {
    const named = transcriptNamedOnCreate(payload.agentType, payload.resumeSessionId)
    // Naming a conversation that is already running: show what is writing it
    // rather than starting a second agent on it, as a resume does.
    const running = sessionToBindOnCreate(named, ptyManager.getLiveSessions())
    if (running) return running
    const session = ptyManager.createPty(payload)
    // Only until the session names the conversation itself: an agent that can be
    // told an id already carries it, and one that cannot reports seconds later.
    if (named && !session.agentSessionId) claimSpawningTranscript(named, session.id)
    return session
  })
  /**
   * Let go of what was kept for a session from the last run.
   *
   * The screen the server rebuilt and the files it rebuilt it from. Both go
   * together, whether the record was claimed or declined -- a live session opens
   * its own history rather than appending to a record of the one it replaced,
   * and a declined one is not coming back.
   */

  registerMethod('terminal:kill', (id) => {
    // A pane showing a session from the last run has no PTY to kill. Closing it
    // is a decision about the record and the files, and it is the same decision
    // resume makes -- so it goes through the same door, and a second client
    // closing the same pane finds nothing rather than an error.
    if (consumeRestored(id)) {
      // Nothing follows, so the caller does not wait on the files going --
      // but a rejection here still has to land somewhere.
      void forgetRestored(id).catch((err) => {
        log.warn({ err, id }, '[restored] could not forget a session that was closed')
      })
      // The row is only removed by a save, and saves are event-driven. Without
      // this, closing a restored pane and quitting leaves the record behind --
      // and the next start offers a session whose files have gone, as an empty
      // pane the person already closed.
      sessionManager.scheduleSave()
      return
    }
    ptyManager.killPty(id)
  })
  registerMethod('terminal:listActive', () => ptyManager.getActiveSessions())
  registerMethod('terminal:rename', ({ id, displayName }) => {
    ptyManager.renameSession(id, displayName)
    logSessionEvent(id, 'renamed', { displayName })
    sessionManager.scheduleSave()
    broadcastWidgetUpdate()
  })
  registerMethod('terminal:reorder', (ids) => {
    ptyManager.reorderSessions(ids)
    sessionManager.scheduleSave()
    broadcastWidgetUpdate()
  })
  /**
   * The board without the rest of the configuration around it.
   *
   * `config:load` carries every task inline, so a client that wants the board
   * pulls roughly a hundred kilobytes and a client that moves one card sends all
   * of it back. The database already stores tasks individually; only the wire
   * treated them as one object.
   */
  registerMethod('task:list', (params) => {
    const filter = (params ?? {}) as {
      projectName?: string
      status?: TaskStatus
      includeDescription?: boolean
    }
    const tasks = dbListTasks(filter.projectName, filter.status)
    if (filter.includeDescription) return tasks
    // A board renders titles. Descriptions are most of the bytes and none of
    // what is drawn, so they are left out until something asks for one task.
    return tasks.map((task) => ({ ...task, description: '' }))
  })

  registerMethod('workflow:resolveGate', ({ runId, nodeId, decision }) => {
    // Broadcast rather than claimed, for the same reason stopping a run is: the
    // client that answers is not necessarily the one holding the run, and on a
    // phone it never is.
    log.info({ runId, nodeId, decision }, '[workflow] broadcasting a gate decision')
    clientRegistry.broadcast(IPC.WORKFLOW_GATE_RESOLVED, { runId, nodeId, decision })
    return { accepted: true }
  })

  registerMethod(
    'workflow:get',
    ({ id }) => configManager.loadConfig().workflows?.find((w) => w.id === id) ?? null
  )

  // Straight from the table, the way `task:list` reads `dbListTasks` rather than
  // going through the configuration. `workflow:get` above still goes the other
  // way and finds by id in a loaded config; that is heavier and inconsistent,
  // and untangling it is not this change.
  registerMethod('workflow:list', () => dbListWorkflows())

  registerMethod('workflow:setEnabled', ({ id, enabled }) => {
    // The row count is how an unknown id is answered. Reading the workflow first
    // would parse its nodes and edges to learn a boolean, so one malformed row
    // could throw a call that never needed the definition -- and it would leave
    // a gap between the check and the write.
    if (dbUpdateWorkflow(id, { enabled }) === 0) return { ok: false }
    // The desktop is drawing this workflow's dot right now and holds the
    // configuration in a cache. Without this it goes on showing the old state
    // until something else invalidates it.
    configManager.notifyChanged()
    return { ok: true }
  })

  registerMethod('project:list', () => configManager.loadConfig().projects ?? [])

  registerMethod('task:get', ({ id }) => dbGetTask(id))

  registerMethod('task:setStatus', ({ id, status }) => {
    const task = dbGetTask(id)
    if (!task) return { ok: false }
    // `dbUpdateTask` writes `updated_at` only when it is handed one, so moving a
    // card between columns used to leave the row claiming it had not been
    // touched since whenever it was last edited.
    dbUpdateTask(id, {
      status,
      updatedAt: new Date().toISOString(),
      ...terminalStamps(task.status, status)
    })
    // Everything else reads the board through the cached config, so a direct
    // row write has to invalidate it. This also broadcasts `config:changed`,
    // which is how other clients learn the card moved.
    configManager.notifyChanged()
    return { ok: true }
  })

  /**
   * Writing a task, not only reading and moving one.
   *
   * Until these existed the only way to write a task over this socket was
   * `config:save` carrying the whole configuration — which is what the MCP
   * tools still do, and what `task:list` was added to stop the board doing.
   * Every one of them ends in `notifyChanged`, because a row written without
   * it is a row no other client hears about.
   */
  registerMethod(
    'task:create',
    ({ projectName, title, description, status, branch, useWorktree, assignedAgent }) => {
      // A task in a project that does not exist is a task nothing can ever run.
      if (!dbGetProject(projectName)) return { ok: false }

      const now = new Date().toISOString()
      const settled = status ?? 'todo'
      const task: TaskConfig = {
        id: crypto.randomUUID(),
        projectName,
        title,
        description: description ?? '',
        status: settled,
        order: dbGetMaxTaskOrder(projectName) + 1,
        createdAt: now,
        updatedAt: now,
        ...(branch && { branch }),
        ...(useWorktree && { useWorktree }),
        ...(assignedAgent && { assignedAgent }),
        ...(isTerminalTaskStatus(settled) && { completedAt: now })
      }
      dbInsertTask(task)
      configManager.notifyChanged()
      // Returned whole: the caller does not have to guess the id it was given
      // or the order it landed at.
      return { ok: true, task }
    }
  )

  /**
   * Named one by one, never spread.
   *
   * The params type is erased at run time — `registerMethod` hands the handler
   * whatever JSON arrived — so a rest spread would put every key a client cared
   * to send into `dbUpdateTask`, whose column whitelist is wider than what this
   * method advertises. That is not a style point: `archivedAt` is one of the two
   * columns `dbUpdateTask` writes on mere presence, so `{ id, archivedAt }`
   * would file away a task that is still open, walking past the terminal-status
   * rule `task:archive` enforces a few lines below. Naming the six fields is
   * what makes that unreachable.
   *
   * `dbUpdateTask` already skips any of these that is `undefined`, so an unsent
   * field needs no guard here. The dates are not a caller's to set: they come
   * from `terminalStamps` or not at all.
   *
   * `projectName` carries two rules of its own, both borrowed from `task:create`.
   * A project that does not exist is refused, because a task in one is a task
   * nothing can run. And `order` is recomputed, because it is per-project: a
   * task carried across keeps a place that means nothing where it lands, and
   * from order 0 in one board into a board that already has an order 0 it lands
   * on top of something. Nothing in the schema forbids that duplicate and
   * `task:reorder` preserves it faithfully, since it permutes the orders already
   * present rather than renumbering them. A moved task goes to the end, exactly
   * as a new one does.
   */
  registerMethod(
    'task:update',
    ({ id, projectName, title, description, status, branch, useWorktree, assignedAgent }) => {
      const task = dbGetTask(id)
      if (!task) return { ok: false }

      const moving = projectName !== undefined && projectName !== task.projectName
      if (moving && !dbGetProject(projectName)) return { ok: false }

      dbUpdateTask(id, {
        projectName,
        ...(moving && { order: dbGetMaxTaskOrder(projectName) + 1 }),
        title,
        description,
        status,
        branch,
        useWorktree,
        assignedAgent,
        updatedAt: new Date().toISOString(),
        ...(status ? terminalStamps(task.status, status) : {})
      })
      configManager.notifyChanged()
      return { ok: true, task: dbGetTask(id) ?? undefined }
    }
  )

  registerMethod('task:delete', ({ id }) => {
    if (!dbGetTask(id)) return { ok: false }
    dbDeleteTask(id)
    configManager.notifyChanged()
    return { ok: true }
  })

  /**
   * The ids in the order they should now sit. Anything absent keeps its place.
   *
   * The places these tasks already occupy are collected and handed back out in
   * the order asked for, rather than numbering the list 0..n. Numbering would
   * make that last sentence false twice over: half a board sent for reordering
   * would be given orders that collide with the half that was not mentioned,
   * and an id naming nothing would still eat a place, pushing everything after
   * it down by one. A permutation cannot do either — the set of orders comes
   * out the same as it went in.
   */
  registerMethod('task:reorder', ({ ids }) => {
    // Deduplicated first, keeping the place each id was first named. An id sent
    // twice would otherwise put its task in the list twice and its order into
    // the slots twice, and the second copy is a place no task can take up: the
    // extra slot pushes a later task onto an order another one already holds.
    const named = [...new Set(ids)]
      .map((id) => dbGetTask(id))
      .filter((task): task is TaskConfig => !!task)
    if (named.length === 0) return { ok: false }

    const slots = named.map((task) => task.order).sort((a, b) => a - b)
    const now = new Date().toISOString()
    let moved = 0
    named.forEach((task, index) => {
      const slot = slots[index] ?? task.order
      if (slot === task.order) return
      dbUpdateTask(task.id, { order: slot, updatedAt: now })
      moved += 1
    })
    // A list already in the order it asks for is a request that succeeded and
    // wrote nothing. Broadcasting there would make every client rebuild a board
    // that did not move.
    if (moved > 0) configManager.notifyChanged()
    return { ok: true }
  })

  registerMethod('task:archive', ({ id, archived }) => {
    const task = dbGetTask(id)
    if (!task) return { ok: false }
    // The same rule `archive_task` enforces on the MCP side. Archiving is for
    // work that is over; a todo hidden from the board is a todo that is lost.
    if (archived && !isTerminalTaskStatus(task.status)) return { ok: false }

    const now = new Date().toISOString()
    dbUpdateTask(id, { archivedAt: archived ? now : undefined, updatedAt: now })
    configManager.notifyChanged()
    return { ok: true }
  })

  registerMethod('terminal:readScrollback', ({ id }) => ({ data: readScrollback(id) }))
  // Read in one tick on purpose: the scrollback and the flush counter move
  // together inside `flushBuffer`, so taking both in the same turn is what
  // guarantees the caller can trust one against the other.
  registerMethod('terminal:attach', ({ id }) => ({
    data: readScrollback(id),
    seq: ptyManager.lastFlushSeq(id),
    live: ptyManager.hasLivePty(id)
  }))
  registerMethod('terminal:readOutput', ({ id, lines }) => ptyManager.getOutput(id, lines))
  registerMethod('shell:create', (cwd) => {
    const session = ptyManager.createShellPty(cwd)
    clientRegistry.broadcast(IPC.SESSION_CREATED, session)
    logSessionEvent(session.id, 'created', {
      agentType: session.agentType,
      projectName: session.projectName,
      projectPath: session.projectPath
    })
    sessionManager.scheduleSave()
    broadcastWidgetUpdate()
    return session
  })

  // Config
  registerMethod('config:load', () => configManager.loadConfig())
  registerMethod('config:save', (config) => {
    clearAgentDetectionCache()
    configManager.saveConfig(config)
    configManager.notifyChanged()
  })

  // Sessions
  registerMethod('sessions:clear', () => {
    // The offer is being declined for all of them at once. Same rule as closing
    // one: the record goes and so does what was written for it.
    for (const one of consumeAllRestored()) {
      void forgetRestored(one.session.id).catch((err) => {
        log.warn({ err, id: one.session.id }, '[restored] could not forget a declined session')
      })
    }
    sessionManager.clear()
  })

  registerMethod('sessions:restored', () => listRestored())

  /**
   * What goes between the run that ended and the run taking its place.
   *
   * Leave the alternate screen, soft reset, default attributes, cursor shown,
   * then a line of its own. A soft reset (DECSTR) and deliberately not a full
   * one: RIS would clear the scrollback, and the scrollback is the thing being
   * resumed. Without it the new process inherits the last one's scroll region,
   * origin mode and unclosed attributes -- it assumes a terminal at its defaults
   * and so never sets them -- and its first redraw lands inside the old frame.
   */
  const BETWEEN_RUNS = '\x1b[?1049l\x1b[!p\x1b[0m\x1b[?25h\r\n'

  registerMethod('sessions:resume', async ({ id }) => {
    // Claimed before anything is started, and that ordering is the point. Two
    // clients can be looking at the same cold pane; the second must be told it
    // is gone rather than launching a second agent against one transcript.
    // Two kinds of ended session, and only one of them is held here.
    //
    // A session carried over from a previous run is in `held`. One that exited
    // during *this* run is not: its record outlives its process in the pty
    // manager, which is what `hasLivePty` exists to tell apart. That second kind
    // is the common one -- an agent finishing its turn -- and it was answered
    // `gone`, which the pane reported as "resumed somewhere else" before
    // deleting itself and its scrollback.
    const restored = consumeRestored(id)
    const dead = restored
      ? undefined
      : ptyManager.getActiveSessions().find((s) => s.id === id && !ptyManager.hasLivePty(id))
    const previous = restored?.session ?? dead
    if (!previous) return { ok: false as const, reason: 'gone' as const }

    const live = ptyManager.getLiveSessions()
    let transcriptId: string | undefined
    const pinned = previous.agentSessionId
    const holder = pinned ? transcriptHolder(pinned, live) : undefined
    if (holder) {
      // Its conversation is already running; hand back what is writing it.
      if (dead) ptyManager.releaseForResume(id)
      await forgetRestored(id)
      sessionManager.scheduleSave()
      return { ok: true as const, session: holder, boundTo: holder.id }
    }

    try {
      // Claimed, for the second kind. Not `killPty`: that announces an exit for
      // a session which is coming straight back, offers to delete the worktree
      // this is about to resume into, and removes the history directory
      // `startHistory` resets moments later.
      if (dead) ptyManager.releaseForResume(id)

      if (previous.agentType === 'shell') {
        // The remembered directory only if it is still a directory. It was
        // reported by the shell over the tty, so anything that could write to
        // that pane could have written it -- and a resume is the one moment it
        // turns into where a process starts. A stale or fabricated path falls
        // back to the project rather than being spawned into.
        const remembered = previous.shellCwd
        // One question, asked once, and never allowed to throw. Asking whether it
        // exists and then whether it is a directory is two questions with a gap
        // between them: a directory removed in that gap turns a fallback into a
        // rejected resume, and the fallback is the whole point of asking.
        const usable = remembered !== undefined && isDirectory(remembered)
        const cwd = usable ? remembered : (previous.worktreePath ?? previous.projectPath)
        // The same id, which is what makes this the same pane rather than a new
        // one beside it: the client keys its terminal by this, so a fresh id
        // would hand back a blank shell and drop the screen being resumed. The
        // previous run's history is not deleted here either -- `startHistory`
        // resets it under this name, on the queue that owns it, and only once
        // something is actually running.
        const session = ptyManager.createShellPty(cwd, id)
        // Carried across on the server rather than grafted on by whichever
        // client asked. `createShellPty` names a session after its directory, so
        // without this a restored shell loses the project it belonged to -- which
        // it does today, for exactly this reason.
        Object.assign(session, {
          projectName: previous.projectName,
          projectPath: previous.projectPath,
          ...(previous.worktreePath !== undefined && { worktreePath: previous.worktreePath }),
          ...(previous.worktreeName !== undefined && { worktreeName: previous.worktreeName }),
          ...(previous.branch !== undefined && { branch: previous.branch }),
          ...(previous.isWorktree !== undefined && { isWorktree: previous.isWorktree }),
          ...(previous.displayName !== undefined && { displayName: previous.displayName })
        })
        // Synchronously, so it is in the buffer before the shell's first byte.
        ptyManager.injectOutput(session.id, BETWEEN_RUNS)
        clientRegistry.broadcast(IPC.SESSION_CREATED, session)
        sessionManager.scheduleSave()
        return { ok: true as const, session }
      }

      transcriptId = claimTranscriptFor(previous, live, id, headlessManager.getActiveSessions())

      // Same id, same reasons as the shell branch above.
      const session = ptyManager.createPty(buildRestorePayload(previous, transcriptId), id)
      // The record names the conversation now, so the claim standing in for it is
      // spent; leaving it would hold an id the session already reports.
      if (session.agentSessionId) releaseSpawningTranscriptsFor(id)
      ptyManager.injectOutput(session.id, BETWEEN_RUNS)
      sessionManager.scheduleSave()
      return { ok: true as const, session }
    } catch (err) {
      log.warn({ err, id }, '[restored] could not resume this session')
      // Put it back. A claim is destructive on purpose, but a claim whose spawn
      // failed must not be the end of the session. Both kinds, because both were
      // taken: the carried-over record goes back to `restored-sessions`, and the
      // one that ended during this run goes back to the pty manager it came from.
      if (restored) restoreHeld(restored)
      else if (dead) ptyManager.restoreReleased(dead)
      if (transcriptId) releaseSpawningTranscript(transcriptId, id)
      return {
        ok: false as const,
        reason: 'failed' as const,
        message: err instanceof Error ? err.message : String(err)
      }
    }
  })
  registerMethod('sessions:getRecent', (projectPath) => getRecentSessions(projectPath))

  // Resolve remote host by ID
  function resolveRemoteHostById(hostId: string): RemoteHost | undefined {
    const cfg = configManager.loadConfig()
    return cfg.remoteHosts?.find((h) => h.id === hostId)
  }

  // Git — resolve remote host for project or worktree paths
  function resolveRemoteHost(projectPath: string): RemoteHost | undefined {
    const cfg = configManager.loadConfig()
    const project = cfg.projects.find((p) => p.path === projectPath)
    if (!project) return undefined
    const remoteId = getProjectRemoteHostId(project)
    if (!remoteId) return undefined
    return cfg.remoteHosts?.find((h) => h.id === remoteId)
  }

  /** Resolve remote host from any path (project root or worktree subdirectory). */
  function resolveRemoteHostByPath(anyPath: string): RemoteHost | undefined {
    const cfg = configManager.loadConfig()
    for (const project of cfg.projects) {
      if (anyPath === project.path || anyPath.startsWith(project.path + '/')) {
        const remoteId = getProjectRemoteHostId(project)
        if (!remoteId) return undefined
        return cfg.remoteHosts?.find((h) => h.id === remoteId)
      }
      const parentDir = project.path.replace(/\/[^/]+$/, '')
      if (anyPath.startsWith(parentDir + '/.vorn-worktrees/')) {
        const remoteId = getProjectRemoteHostId(project)
        if (!remoteId) return undefined
        return cfg.remoteHosts?.find((h) => h.id === remoteId)
      }
    }
    return undefined
  }

  registerMethod('git:isGitRepo', (projectPath) => gitUtils.isGitRepo(projectPath))
  registerMethod('git:listBranches', (projectPath) => {
    const remote = resolveRemoteHost(projectPath)
    const isRepo = remote || gitUtils.isGitRepo(projectPath)
    return {
      local: isRepo ? gitUtils.listBranches(projectPath, remote) : [],
      current: isRepo ? gitUtils.getGitBranch(projectPath, remote) : null,
      isGitRepo: !!isRepo
    }
  })
  registerMethod('git:listRemoteBranches', (projectPath) => {
    const remote = resolveRemoteHost(projectPath)
    return gitUtils.listRemoteBranches(projectPath, remote)
  })
  registerMethod('git:createWorktree', ({ projectPath, branch, worktreeName }) => {
    const remote = resolveRemoteHost(projectPath)
    return gitUtils.createWorktree(projectPath, branch, worktreeName, remote)
  })
  registerMethod('git:removeWorktree', ({ projectPath, worktreePath, force, deleteBranch }) => {
    const remote = resolveRemoteHost(projectPath)
    invalidateSizeCache(worktreePath)
    return gitUtils.removeWorktree(projectPath, worktreePath, force, remote, deleteBranch)
  })
  registerMethod('git:checkoutBranch', ({ cwd, branch }) => {
    const remote = resolveRemoteHostByPath(cwd)
    const result = gitUtils.checkoutBranch(cwd, branch, remote)
    if (result.ok) {
      ptyManager.updateSessionsForWorktree(cwd, { branch })
      headlessManager.updateSessionsForWorktree(cwd, { branch })
    }
    return result
  })
  registerMethod('git:getWorktreeBranch', (worktreePath) => {
    const remote = resolveRemoteHostByPath(worktreePath)
    return gitUtils.getGitBranch(worktreePath, remote)
  })
  registerMethod('git:renameWorktreeBranch', ({ worktreePath, newBranch }) => {
    const remote = resolveRemoteHostByPath(worktreePath)
    const result = gitUtils.renameWorktreeBranch(worktreePath, newBranch, remote)
    if (result) {
      ptyManager.updateSessionsForWorktree(worktreePath, { branch: newBranch })
      headlessManager.updateSessionsForWorktree(worktreePath, { branch: newBranch })
    }
    return result
  })
  registerMethod('git:renameWorktree', ({ worktreePath, newName }) => {
    const remote = resolveRemoteHostByPath(worktreePath)
    const result = gitUtils.renameWorktree(worktreePath, newName, remote)
    if (result) {
      ptyManager.updateSessionsForWorktree(worktreePath, {
        worktreePath: result.newPath,
        worktreeName: result.name
      })
      headlessManager.updateSessionsForWorktree(worktreePath, {
        worktreePath: result.newPath,
        worktreeName: result.name
      })
    }
    return result
  })
  registerMethod('git:worktreeDirty', (worktreePath) => {
    const remote = resolveRemoteHostByPath(worktreePath)
    return gitUtils.isWorktreeDirty(worktreePath, remote)
  })
  registerMethod('git:listWorktrees', (projectPath) => {
    const remote = resolveRemoteHost(projectPath)
    return gitUtils.listWorktrees(projectPath, remote)
  })

  registerMethod('worktree:activeSessions', (worktreePath: string) => {
    const pty = ptyManager.getActiveSessionsForWorktree(worktreePath)
    const headless = headlessManager.getActiveSessionsForWorktree(worktreePath)
    return {
      count: pty.count + headless.count,
      sessionIds: [...pty.sessionIds, ...headless.sessionIds]
    }
  })

  // ─── Worktree manager ──────────────────────────────────────────

  function activeSessionIds(worktreePath: string): string[] {
    return [
      ...ptyManager.getActiveSessionsForWorktree(worktreePath).sessionIds,
      ...headlessManager.getActiveSessionsForWorktree(worktreePath).sessionIds
    ]
  }

  function retentionConfig(): WorktreeRetentionConfig {
    return configManager.loadConfig().defaults.worktreeRetention ?? {}
  }

  function artifactDirNames(): string[] {
    const configured = retentionConfig().artifactDirs
    return configured?.length ? configured : DEFAULT_ARTIFACT_DIRS
  }

  /** Cached size for a path — measured during the scan that preceded the action. */
  function cachedSizeOf(worktreePath: string): number {
    const remote = resolveRemoteHostByPath(worktreePath)
    return measureWorktree(worktreePath, artifactDirNames(), remote).sizeBytes
  }

  /**
   * Refuse to act on a worktree that has a live session. Checked immediately
   * before the action rather than read off the scan, because a session can
   * start while the panel is open.
   */
  function assertNoActiveSessions(paths: string[]): void {
    for (const p of paths) {
      const count = activeSessionIds(p).length
      if (count > 0) {
        throw new Error(
          `${p} has ${count} active session${count > 1 ? 's' : ''} — close them first`
        )
      }
    }
  }

  /** Resolve a project to its remote host, or undefined when it is local. */
  function remoteForProject(project: ProjectConfig): RemoteHost | undefined {
    const remoteId = getProjectRemoteHostId(project)
    if (!remoteId) return undefined
    return configManager.loadConfig().remoteHosts?.find((h) => h.id === remoteId)
  }

  registerMethod('worktree:inventory', (params) => {
    const cfg = configManager.loadConfig()
    return scanWorktreeInventory({
      projects: cfg.projects,
      projectPaths: params?.projectPaths,
      refresh: params?.refresh,
      retention: cfg.defaults.worktreeRetention,
      resolveRemote: remoteForProject,
      getActiveSessions: activeSessionIds
    })
  })

  registerMethod('worktree:reclaimArtifacts', ({ paths }) => {
    assertNoActiveSessions(paths)
    const cfg = configManager.loadConfig()
    return reclaimArtifacts(paths, artifactDirNames(), cfg.projects, remoteForProject)
  })

  registerMethod('worktree:removeMany', ({ items }) => {
    assertNoActiveSessions(items.map((i) => i.worktreePath))
    const cfg = configManager.loadConfig()
    return removeWorktrees(items, cachedSizeOf, cfg.projects, remoteForProject)
  })

  registerMethod('worktree:pruneOrphans', ({ paths }) => {
    assertNoActiveSessions(paths)
    return pruneOrphanDirs(paths, cachedSizeOf, resolveRemoteHostByPath)
  })

  registerMethod('git:deleteBranches', ({ projectPath, branches, force }) => {
    const remote = resolveRemoteHost(projectPath)
    return deleteStaleBranches(projectPath, branches, force ?? false, remote)
  })
  registerMethod('git:getBranch', (cwd) => {
    const remote = resolveRemoteHostByPath(cwd)
    return gitUtils.getGitBranch(cwd, remote)
  })
  registerMethod('git:diffStat', (cwd) => {
    const remote = resolveRemoteHostByPath(cwd)
    return gitUtils.getGitDiffStat(cwd, remote)
  })
  registerMethod('git:diffFull', (cwd) => {
    const remote = resolveRemoteHostByPath(cwd)
    return gitUtils.getGitDiffFull(cwd, remote)
  })
  registerMethod('git:commit', ({ cwd, message, includeUnstaged }) => {
    const remote = resolveRemoteHostByPath(cwd)
    return gitUtils.gitCommit(cwd, message, includeUnstaged, remote)
  })
  registerMethod('git:push', (cwd) => {
    const remote = resolveRemoteHostByPath(cwd)
    return gitUtils.gitPush(cwd, remote)
  })

  // Scheduler
  registerMethod('scheduler:getLog', (workflowId) => scheduleLogManager.getEntries(workflowId))
  registerMethod('scheduler:getNextRun', (workflowId) => {
    const config = configManager.loadConfig()
    return scheduler.getNextRun(workflowId, config.workflows ?? [])
  })

  // Task images
  registerMethod('task:imageSave', ({ taskId, sourcePath }) => saveTaskImage(taskId, sourcePath))
  registerMethod('task:imageDelete', ({ taskId, filename }) => deleteTaskImage(taskId, filename))
  registerMethod('task:imageGetPath', ({ taskId, filename }) => getTaskImagePath(taskId, filename))
  registerMethod('task:imageCleanup', (taskId) => cleanupTaskImages(taskId))
  registerMethod('task:imageUpload', ({ taskId, base64, filename }) =>
    saveTaskImageFromBase64(taskId, base64, filename)
  )

  // Headless
  registerMethod('headless:create', (payload) => {
    const session = headlessManager.createHeadless(payload)
    logSessionEvent(session.id, 'created', {
      agentType: payload.agentType,
      projectName: payload.projectName,
      projectPath: payload.projectPath,
      headless: true
    })
    return session
  })
  registerMethod('headless:kill', (id) => headlessManager.killHeadless(id))
  registerMethod('headless:list', () => headlessManager.getActiveSessions())

  // Scripts
  registerMethod('script:execute', (config) => executeScript(config))

  // Workflow runs
  registerMethod('workflowRun:save', (execution) => saveWorkflowRun(execution))
  registerMethod('workflowRun:list', ({ workflowId, limit }) => listWorkflowRuns(workflowId, limit))
  registerMethod('workflowRun:listByTask', ({ taskId, limit }) =>
    listWorkflowRunsByTask(taskId, limit)
  )
  registerMethod('workflowRun:listWaiting', () => listRunsWithWaitingGates())
  registerMethod('workflowRun:listRunning', () => listRunningRuns())
  registerMethod('workflowRun:listAll', ({ workspaceId, limit }) =>
    listAllWorkflowRuns(workspaceId, limit)
  )

  // Session events
  registerMethod('sessionEvent:list', ({ eventType, limit }) => listSessionEvents(eventType, limit))
  registerMethod('sessionEvent:listBySession', ({ sessionId, limit }) =>
    listSessionEventsBySession(sessionId, limit)
  )

  // Agent/IDE detection
  registerMethod('agent:detectInstalled', () => detectInstalledAgents())
  registerMethod('ide:detect', () => detectIDEs())
  registerMethod('project:detectMobile', ({ projectPath }) => detectMobileProject(projectPath))
  registerMethod('ide:open', ({ ideId, projectPath }) => openInIDE(ideId, projectPath))

  // Where a browser can reach this server. Asked separately from Tailscale status
  // because it has to answer even when Tailscale is absent — that is the case the
  // old UI could not express at all.
  registerMethod('server:reachableUrls', async () => {
    let tailscaleIps: string[] = []
    try {
      const status = await getTailscaleStatus()
      if (status.running && status.selfIP) tailscaleIps = [status.selfIP]
    } catch {
      // Not installed or not answering; LAN addresses still stand.
    }
    return reachableUrls(serverPort, tailscaleIps)
  })

  registerMethod('webhook:info', () => ({ baseUrl: `http://127.0.0.1:${serverPort}` }))

  // Tailscale network access. Informational only now: it supplies an address and
  // a QR code, and no longer decides whether the server binds wide.
  registerMethod('tailscale:status', async () => {
    clearBinaryCache() // Always re-detect in case user just installed
    // Deliberately does not rebind. Reading status used to have that side effect,
    // because Tailscale could start after boot and change the answer. Nothing
    // about the bind depends on it now, and rebinding drops every connection —
    // so it happens when the setting changes, and at no other time.
    return getTailscaleStatus(serverPort)
  })

  // Credential vault (storage — encryption handled by main process)
  registerMethod('credential:storeKey', (params) => {
    const id = crypto.randomUUID()
    dbSaveSSHKey({
      id,
      label: params.label,
      encryptedPrivateKey: params.encryptedPrivateKey,
      publicKey: params.publicKey,
      certificate: params.certificate,
      keyType: params.keyType,
      createdAt: new Date().toISOString()
    })
    return { id }
  })
  registerMethod('credential:listKeys', () => dbListSSHKeys())
  registerMethod('credential:deleteKey', (id) => dbDeleteSSHKey(id))
  registerMethod('credential:getEncryptedKey', (id) => dbGetSSHKey(id))

  // Device tokens. Until now these existed only behind `vorn-server token`, so
  // pairing a phone meant finding a terminal on the machine running the server.
  registerMethod('token:list', () => listTokens())
  registerMethod('token:create', ({ name }) => {
    // Coerced rather than trusted: a malformed param would otherwise fail inside
    // `.trim()` with a TypeError that reaches the client verbatim, saying nothing
    // about what was wrong.
    const label = typeof name === 'string' ? name.trim() : ''
    // The plaintext is returned exactly once and never stored — only its hash
    // reaches the database — so the caller has to show it and then drop it.
    const minted = mintOwnerToken(label || 'Device')
    return { token: minted.token, plaintext: minted.plaintext }
  })
  // Pairing a phone. These four are the desktop's half: it asks for a code,
  // sees who offered it, and decides. The phone's half is two HTTP routes in
  // `index.ts`, because a phone that has not paired yet has no credential and
  // the socket admits exactly one method before authenticating.
  registerMethod('pairing:start', () => startPairing())
  registerMethod('pairing:pending', () => pendingRequests())
  registerMethod('pairing:approve', ({ requestId }) => ({ ok: approveRequest(requestId) }))
  registerMethod('pairing:deny', ({ requestId }) => ({ ok: denyRequest(requestId) }))
  registerMethod('pairing:cancel', () => {
    cancelPairing()
    return { ok: true }
  })

  registerMethod('token:revoke', (id) => {
    const revoked = revokeToken(id)
    // Revoking has to reach a socket already holding the token, or a lost phone
    // keeps working until it happens to reconnect.
    if (revoked) disconnectToken(id)
    return { revoked }
  })

  // File explorer
  registerMethod('file:listDir', ({ dirPath, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return listDir(dirPath, remote)
  })
  registerMethod('file:readContent', ({ filePath, maxBytes, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return readFileContent(filePath, maxBytes, remote)
  })
  registerMethod('file:stamp', ({ filePath, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return fileStamp(filePath, remote)
  })
  registerMethod('file:writeContent', ({ filePath, content, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return writeFileContent(filePath, content, remote)
  })

  // Intent bar completions
  registerMethod('shell:listExecutables', () => listShellExecutables())
  registerMethod('shell:listInstalled', () => listInstalledShells())

  // SSH
  registerMethod('ssh:testConnection', (host) => testSshConnection(host))

  // Fire-and-forget notifications
  registerNotification('terminal:write', ({ id, data }) => ptyManager.writeToPty(id, data))
  registerNotification('terminal:resize', ({ id, cols, rows }) =>
    ptyManager.resizePty(id, cols, rows)
  )

  // Permission resolution
  registerMethod('permission:resolve', ({ requestId, allow, updatedPermissions, updatedInput }) => {
    hookServer.resolvePermission(requestId, allow, { updatedPermissions, updatedInput })
  })

  // Resolve top pending permission (for global shortcuts)
  registerMethod('permission:resolve-top', ({ allow }) => {
    const pending = hookServer.getPendingPermissions()
    if (pending.length > 0) {
      hookServer.resolvePermission(pending[0].requestId, allow)
    }
  })

  // Widget status update request
  registerMethod('widget:requestUpdate', () => {
    broadcastWidgetUpdate()
  })

  // Workflow execution complete
  registerMethod(
    'workflow:executionComplete',
    (data: {
      workflowId: string
      workflowName: string
      completedAt: string
      status: 'success' | 'error' | 'cancelled'
      sessionsLaunched: number
      source?: 'scheduler' | 'manual'
    }) => {
      if (data.status !== 'success' && data.status !== 'error' && data.status !== 'cancelled') {
        return
      }
      // A run the user stopped isn't a schedule outcome — it still updates the
      // workflow's last-run badge, but it would misreport the schedule's health.
      if (data.source === 'scheduler' && data.status !== 'cancelled') {
        scheduleLogManager.addEntry({
          workflowId: data.workflowId,
          workflowName: data.workflowName,
          executedAt: data.completedAt,
          status: data.status,
          sessionsLaunched: data.sessionsLaunched
        })
      }
      updateWorkflowRunStatus(data.workflowId, data.completedAt, data.status)
      configManager.notifyChanged()
    }
  )

  // Connectors
  registerMethod('connector:list', () => {
    return connectorRegistry.list().map((c) => ({
      id: c.id,
      name: c.name,
      icon: c.icon,
      capabilities: [...c.capabilities],
      manifest: c.describe()
    }))
  })

  registerMethod('connector:get', (id) => {
    const c = connectorRegistry.get(id)
    if (!c) return null
    return {
      id: c.id,
      name: c.name,
      icon: c.icon,
      capabilities: [...c.capabilities],
      manifest: c.describe()
    }
  })

  registerMethod('connection:list', ({ connectorId }) => {
    // The internal webhook row only satisfies the inbox's connection reference.
    return dbListSourceConnections(connectorId).filter((c) => c.connectorId !== 'webhook')
  })

  registerMethod('connection:create', (params) => createConnectionRecord(params))

  registerMethod('connection:update', async ({ id, updates }) => {
    dbUpdateSourceConnection(id, updates)
    // Awaited, so an action issued right after an edit cannot still be served
    // by the child holding the command this edit replaced.
    await stopMcpClient(id).catch((err) => log.warn(`[mcp] stopClient failed: ${err}`))
    dbSignalChange()
    return dbGetSourceConnection(id)
  })

  registerMethod('connection:delete', (id) => {
    const conn = dbGetSourceConnection(id)
    // It would come straight back on the next pack change; the pack is the thing to remove.
    if (conn && isImplicitConnection(conn)) {
      throw new Error(`${conn.name} came with its connector. Remove the pack instead.`)
    }
    deleteConnectionRecord(id)
  })

  registerMethod('workflow:runManual', ({ workflowId, inputs }) => {
    const wf = dbGetWorkflow(workflowId)
    if (!wf) throw new Error(`Workflow ${workflowId} not found`)
    // Refuse a startless run here rather than burning a claim on a fake run.
    if (!wf.nodes.some((n) => n.type === 'trigger')) {
      throw new Error(`Workflow "${wf.name}" has no trigger; add one before running it`)
    }
    scheduler.triggerWorkflow(workflowId, inputs)
  })

  // Runs live in the renderer, so stopping one is a request broadcast to every
  // connected instance rather than something this process can do itself. The
  // instance owning the run recognises the id and tears it down; the others
  // find no such run and ignore it.
  registerMethod('workflow:stopRun', ({ runId }: { runId: string }) => {
    scheduler.stopRun(runId)
  })

  registerMethod('connector:inboxComplete', ({ id, leaseToken, disposition, error }) => {
    scheduler.completeConnectorInbox(id, leaseToken, disposition, error)
  })

  registerMethod('connector:inboxRenew', ({ id, leaseToken }) => {
    return scheduler.renewConnectorInbox(id, leaseToken)
  })

  // Runs execute in the renderer, but a scheduler tick reaches every connected
  // instance. Claiming here — in the one process they all share — is what stops
  // two open windows from launching the same agents twice.
  registerMethod('workflowRun:claim', (req: RunClaimRequest) => claimWorkflowRun(req))

  registerMethod(
    'workflowRun:release',
    ({ workflowId, params, runId }: { workflowId: string; params?: string; runId: string }) => {
      releaseWorkflowRun(workflowId, params, runId)
    }
  )

  registerMethod('credentials:setDecrypted', ({ connectionId, fields }) => {
    setDecryptedCreds(connectionId, fields)
  })

  registerMethod('credentials:clearDecrypted', ({ connectionId }) => {
    clearDecryptedCreds(connectionId)
  })

  registerMethod('http:request', async ({ profileConnectionId, method, url, headers, body }) => {
    let profile: Record<string, unknown> = {}
    if (profileConnectionId) {
      const conn = dbGetSourceConnection(profileConnectionId)
      if (!conn) return { success: false, error: `Connection ${profileConnectionId} not found` }
      const problem = httpProfileError(conn, getDecryptedCreds(conn.id))
      if (problem) return { success: false, error: problem }
      profile = applyDecryptedCreds(conn)
    }
    return performHttpRequest(profile, { method, url, headers, body })
  })

  registerMethod('connection:listKeys', () => {
    const auth = new Map<string, ConnectorConfigField[] | undefined>(
      connectorRegistry.list().map((c) => [c.id, c.describe().auth])
    )
    return listKeys(
      dbListSourceConnections().filter((c) => c.connectorId !== 'webhook'),
      auth,
      dbListWorkflows(),
      getDecryptedCreds
    )
  })

  /**
   * Replace one stored secret.
   *
   * The ciphertext is what persists; the plaintext is adopted in the same call
   * rather than waited for. Clearing and letting the desktop process push the
   * new value back left a window — brief, but long enough for a poll or an
   * action to run against a connection whose key had become unreadable.
   * Ending the child is part of the write too: one left running would keep
   * serving the key that was just rotated away.
   */
  registerMethod('connection:rotateSecret', async ({ connectionId, field, value, plaintext }) => {
    const conn = dbGetSourceConnection(connectionId)
    if (!conn) return { ok: false, error: `connection ${connectionId} not found` }
    const connector = connectorRegistry.get(conn.connectorId)
    const secret = passwordFields(connector?.describe().auth).some((f) => f.key === field)
    if (!secret) return { ok: false, error: `${field} is not a secret on this connection` }
    // A blank replacement is a mistake rather than an intent to unset: it would
    // leave every workflow bound to this key failing with nothing said.
    if (value === '' || plaintext.trim() === '') {
      return { ok: false, error: 'A replacement value is required' }
    }

    dbUpdateSourceConnection(connectionId, { filters: { ...conn.filters, [field]: value } })
    // Merged, not replaced: a connection can hold more than one secret, and the
    // others are still live.
    setDecryptedCreds(connectionId, { ...getDecryptedCreds(connectionId), [field]: plaintext })
    await stopMcpClient(connectionId).catch((err) => log.warn(`[mcp] stopClient failed: ${err}`))
    dbSignalChange()
    // The desktop process re-derives the same value from the keychain on this
    // broadcast, which is a confirmation rather than the thing being waited on.
    configManager.notifyChanged()
    return { ok: true }
  })

  registerMethod('connection:executeAction', async ({ connectionId, action, args }) => {
    const conn = dbGetSourceConnection(connectionId)
    if (!conn) return { success: false, error: `Connection ${connectionId} not found` }

    // MCP connections route through invokeMcpTool because the tool call needs
    // the SourceConnection itself (to start / address the per-connection stdio
    // client), not just the merged args the generic execute path provides.
    if (conn.connectorId === MCP_CONNECTOR_ID) {
      return invokeMcpTool(conn, action, args ?? {})
    }

    const connector = connectorRegistry.get(conn.connectorId)
    if (!connector?.execute) {
      return {
        success: false,
        error: `Connector ${conn.connectorId} does not support actions`
      }
    }
    if (conn.connectorId === 'http') {
      const locked = lockedProfileError(conn.filters, getDecryptedCreds(conn.id))
      if (locked) return { success: false, error: locked }
    }
    // Merge auth (from decrypted store) + connection filters + call-specific args.
    // Call args take precedence so users can override e.g. repo per-call.
    const mergedArgs = { ...applyDecryptedCreds(conn), ...args }
    try {
      return await connector.execute(action, mergedArgs)
    } catch (err) {
      return {
        success: false,
        error: err instanceof Error ? err.message : String(err)
      }
    }
  })

  /**
   * Return the actions a connection exposes, in the same `ConnectorActionDef`
   * shape regardless of connector type. Static connectors return their
   * manifest actions verbatim; MCP maps its per-connection discovered tools
   * to the same shape. The workflow editor drives its Action picker off this
   * endpoint so the form stays connector-agnostic.
   */
  registerMethod('connection:listActions', (connectionId: string) => {
    const conn = dbGetSourceConnection(connectionId)
    if (!conn) return []
    if (conn.connectorId === MCP_CONNECTOR_ID) return mcpConnectionActions(conn)
    const connector = connectorRegistry.get(conn.connectorId)
    return connector?.describe().actions ?? []
  })

  registerMethod('connection:listMcpTools', (connectionId: string) => {
    const conn = dbGetSourceConnection(connectionId)
    if (!conn || conn.connectorId !== MCP_CONNECTOR_ID) return []
    // The console shows what the connector offers, matching every other list.
    return visibleMcpTools(conn)
  })

  registerMethod('connection:refreshMcpTools', async (connectionId: string) => {
    return runMcpDiscovery(connectionId)
  })

  /**
   * Ask a packaged connection whether it could run right now.
   *
   * `ok: null` means the connector declares no preflight, which is most of
   * them — only the ones borrowing an external tool's login have anything to
   * check. A connector that authenticates from config fields already fails
   * visibly when a field is missing.
   */
  registerMethod('connection:preflight', async (connectionId: string) => {
    const conn = dbGetSourceConnection(connectionId)
    // A connection that does not exist is a caller error, not an answer about
    // readiness. Returning `ok: null` for it would file a stale or mistyped id
    // under "nothing to check" — the one reading a user is most likely to take
    // as reassurance.
    if (!conn) throw new Error(`connection ${connectionId} not found`)
    // An http profile's preflight is a real request through its injection.
    if (conn.connectorId === 'http') {
      const locked = lockedProfileError(conn.filters, getDecryptedCreds(conn.id))
      if (locked) return { ok: false, message: locked }
      const result = await httpConnector.execute!('test', applyDecryptedCreds(conn))
      const status = (result.output as { status?: number } | undefined)?.status
      if (!result.success) return { ok: false, message: result.error }
      return { ok: (status ?? 500) < 400, message: `HTTP ${status}` }
    }
    // A built-in connector genuinely declares no preflight, so this really is
    // "nothing to check".
    if (conn.connectorId !== MCP_CONNECTOR_ID) return { ok: null }
    try {
      return await preflightMcpConnection(conn)
    } catch (err) {
      // Starting the connector at all is itself part of what preflight
      // answers: a package that will not launch is exactly the state the
      // caller wants reported, not an exception to handle.
      return { ok: false, message: err instanceof Error ? err.message : String(err) }
    }
  })

  /**
   * Read a connector package's self-description before any connection exists,
   * so the connection form can be filled in from the manifest rather than
   * transcribed from a README.
   */
  registerMethod('connector:probeSdk', async (request: SdkProbeRequest) => {
    return probeSdkConnector(request)
  })

  /**
   * Connector packages Vorn knows by name, so a first-party connector can be
   * offered in the connector list rather than requiring its package name.
   */
  registerMethod('connector:catalog', () => catalogSnapshot())

  /**
   * Fetch the published catalog now rather than waiting for the next stale
   * check, so someone who just heard a connector exists can go and find it.
   */
  registerMethod('connector:catalogRefresh', async () => {
    await refreshCatalog()
    return catalogSnapshot()
  })

  /** Verify and describe a pack without keeping any of it, so an install can be confirmed. */
  registerMethod('connector:inspectPack', (source: ConnectorPackSource) => {
    const refusal = unusableSource(source)
    return refusal ? { ok: false as const, error: refusal } : inspectPack(source)
  })

  /** Progress is pushed, not returned, so a caller can show the download as it runs. */
  registerMethod('connector:installPack', async (source: ConnectorPackSource) => {
    const refusal = unusableSource(source)
    if (refusal) return { ok: false as const, error: refusal }
    const result = await installPack(source, {
      onProgress: (progress) => clientRegistry.broadcast(IPC.CONNECTOR_INSTALL_PROGRESS, progress),
      onChanged: onPackChanged
    })
    if (result.ok) dbSignalChange()
    return result
  })

  registerMethod('connector:removePack', async (id: string) => {
    // Counted before the files go; the app's own connection goes with the pack, so it is not one.
    const connections = connectionsForConnector(id).filter(
      (conn) => !isImplicitConnection(conn)
    ).length
    const result = await removePack(id, { onChanged: onPackChanged })
    if (result.ok) dbSignalChange()
    return { ...result, connections }
  })

  registerMethod('connector:rollbackPack', async (id: string) => {
    const result = await rollbackPack(id, { onChanged: onPackChanged })
    if (result.ok) dbSignalChange()
    return result
  })

  registerMethod('connector:listPacks', () => listInstalledPacks())

  /**
   * One-shot backfill for a connection. Calls listItems() (not poll()) so it
   * bypasses the "since now" cursor and pulls everything matching the current
   * filters. Uses the same upsert+link logic as the workflow path so field
   * ownership stays consistent.
   */
  registerMethod('connection:backfill', async ({ connectionId }) => {
    const conn = dbGetSourceConnection(connectionId)
    if (!conn) return { imported: 0, updated: 0, error: 'Connection not found' }
    const connector = connectorRegistry.get(conn.connectorId)
    // MCP is polymorphic: draining it needs the full SourceConnection to
    // address the per-connection stdio client, which the generic
    // listItemsPage(filters) signature cannot carry — the same reason the
    // scheduler routes MCP polling through pollMcpConnection.
    const isMcp = conn.connectorId === MCP_CONNECTOR_ID
    if (!isMcp && !connector?.listItems && !connector?.listItemsPage) {
      return {
        imported: 0,
        updated: 0,
        error: `Connector ${conn.connectorId} does not support listItems()`
      }
    }

    let imported = 0
    let updated = 0
    const now = new Date().toISOString()
    const projectName = conn.executionProject || conn.name

    try {
      const drain = isMcp
        ? (visit: (item: ExternalItem) => void) => backfillMcpConnection(conn, visit)
        : (visit: (item: ExternalItem) => void) =>
            forEachConnectorItem(connector!, applyDecryptedCreds(conn), visit)

      await drain((item) => {
        const initialStatus = conn.statusMapping?.[item.status] || ('todo' as TaskStatus)
        const upserted = upsertExternalItem(
          conn,
          {
            externalId: item.externalId,
            title: item.title,
            description: item.description,
            externalUrl: item.url,
            sourceStatusRaw: item.status,
            sourceUpdatedAt: item.updatedAt
          },
          { projectName, initialStatus, now }
        )
        if (upserted.created) imported++
        else updated++
      })
      dbUpdateSourceConnection(conn.id, { lastSyncAt: now, lastSyncError: undefined })
      dbSignalChange()
      configManager.notifyChanged()
      return { imported, updated }
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : String(err)
      dbUpdateSourceConnection(conn.id, { lastSyncError: errorMsg })
      dbSignalChange()
      return { imported, updated, error: errorMsg }
    }
  })

  registerMethod('connection:getSourceLink', (taskId) => {
    return dbGetTaskSourceLink(taskId)
  })

  registerMethod('connection:upsertFromItem', ({ connectionId, item, initialStatus, project }) => {
    const conn = dbGetSourceConnection(connectionId)
    if (!conn) throw new Error(`connection ${connectionId} not found`)

    const now = new Date().toISOString()
    const result = upsertExternalItem(
      conn,
      {
        externalId: item.externalId,
        title: item.title,
        description: item.body ?? '',
        externalUrl: item.externalUrl ?? '',
        sourceStatusRaw: typeof item.raw?.status === 'string' ? item.raw.status : '',
        sourceUpdatedAt: typeof item.raw?.updatedAt === 'string' ? item.raw.updatedAt : now
      },
      {
        projectName: project || conn.executionProject || conn.name,
        initialStatus,
        now
      }
    )
    dbUpdateSourceConnection(conn.id, { lastSyncAt: now })
    dbSignalChange()
    configManager.notifyChanged()
    return result
  })

  registerMethod('connector:detectRepo', (projectPath) => {
    return detectRepoSlug(projectPath)
  })

  registerMethod('connector:seedWorkflow', ({ connectionId, event }) => {
    const conn = dbGetSourceConnection(connectionId)
    if (!conn) throw new Error(`connection ${connectionId} not found`)
    const connector = connectorRegistry.get(conn.connectorId)
    if (!connector) throw new Error(`connector ${conn.connectorId} not registered`)
    const manifest = connector.describe()
    const eventDef = manifest.defaultWorkflows?.find((e) => e.event === event)
    if (!eventDef) throw new Error(`event ${event} not defined by connector ${conn.connectorId}`)

    const wfId = connectorSeededWorkflowId(conn.id, event)
    if (dbGetWorkflow(wfId)) {
      return { workflowId: wfId, created: false }
    }
    const wf = buildConnectorSeededWorkflow(conn, manifest, eventDef)
    dbInsertWorkflow(wf)
    dbSignalChange()
    configManager.notifyChanged()
    return { workflowId: wfId, created: true }
  })

  registerMethod('connector:status', async () => {
    const results: Array<{ connectorId: string; authed: boolean; message?: string }> = []
    const reports = await Promise.all(
      connectorRegistry
        .list()
        .map(async (c) => ({ c, report: await probeAuth(await authForConnector(c.id)) }))
    )
    for (const { c, report } of reports) {
      // null is "nothing this probe can answer", never reported as signed out.
      const authed = report.ok !== false
      const message = [report.message, report.installHint].filter(Boolean).join('\n')
      results.push({ connectorId: c.id, authed, ...(!authed && message && { message }) })
    }
    return results
  })

  // What lets a cli connector's form show an identity instead of a token field.
  registerMethod('connector:probeAuth', async (connectorId: string) =>
    probeAuth(await authForConnector(connectorId))
  )

  // ─── Browser pane (relayed to Electron main) ──────────────────
  //
  // The guest `<webview>` and its CDP debugger only exist in main, so these
  // are the one method family this process does not answer itself. Each is a
  // straight relay over the reverse bridge; the session scoping that makes
  // them safe happens in the MCP layer, which resolves the caller from
  // VORN_SESSION_ID and never accepts a session as an argument.
  registerMethod('browser:readPage', (p) => browserBridge.request('browser:readPage', p))
  registerMethod('browser:getText', (p) => browserBridge.request('browser:getText', p))
  registerMethod('browser:consoleMessages', (p) =>
    browserBridge.request('browser:consoleMessages', p)
  )
  registerMethod('browser:networkRequests', (p) =>
    browserBridge.request('browser:networkRequests', p)
  )
  registerMethod('browser:screenshot', (p) => browserBridge.request('browser:screenshot', p))
  registerMethod('browser:interact', (p) => browserBridge.request('browser:interact', p))
  registerMethod('browser:tabs', (p) => browserBridge.request('browser:tabs', p))
  registerMethod('browser:openPane', (p) => browserBridge.request('browser:openPane', p))
  registerMethod('browser:navigate', (p) => browserBridge.request('browser:navigate', p))
  registerMethod('browser:history', (p) => browserBridge.request('browser:history', p))
  registerMethod('browser:listTabs', (p) => browserBridge.request('browser:listTabs', p))
  registerMethod('browser:find', (p) => browserBridge.request('browser:find', p))

  // Device pane (relayed to Electron main, same bridge, same reasoning: the
  // idb_companion child process and its unix socket live only in main).
  registerMethod('device:list', (p) => browserBridge.request('device:list', p))
  registerMethod('device:claim', (p) => browserBridge.request('device:claim', p))
  registerMethod('device:release', (p) => browserBridge.request('device:release', p))
  registerMethod('device:readScreen', (p) => browserBridge.request('device:readScreen', p))
  registerMethod('device:find', (p) => browserBridge.request('device:find', p))
  registerMethod('device:interact', (p) => browserBridge.request('device:interact', p))
  registerMethod('device:screenshot', (p) => browserBridge.request('device:screenshot', p))
  registerMethod('device:launch', (p) => browserBridge.request('device:launch', p))
  registerMethod('device:terminate', (p) => browserBridge.request('device:terminate', p))
  registerMethod('device:install', (p) => browserBridge.request('device:install', p))
  registerMethod('device:openUrl', (p) => browserBridge.request('device:openUrl', p))
  registerMethod('device:logs', (p) => browserBridge.request('device:logs', p))
  registerMethod('device:openPane', (p) => browserBridge.request('device:openPane', p))

  /**
   * Which instance a manager notification is about, or nothing.
   *
   * `terminal:data` and its siblings all carry the terminal's id, and a client
   * that only wants one terminal's output subscribes to `terminal:data#<id>`.
   * Payloads without an `id` simply have no instance, and match by name alone.
   */
  const terminalScope = (payload: unknown): string | undefined => {
    const id = (payload as { id?: unknown } | null)?.id
    return typeof id === 'string' ? id : undefined
  }

  // Wire manager events → broadcast to WS clients
  ptyManager.on('client-message', (channel: string, payload: unknown) => {
    // A payload's `id` is the instance this notification is about, which lets a
    // client subscribe to one terminal rather than to all of them. Read
    // generically rather than per channel: every id-bearing payload here means
    // the same thing by it.
    clientRegistry.broadcast(channel, payload, terminalScope(payload))
    if (channel === IPC.TERMINAL_EXIT) {
      const p = payload as { id: string; exitCode: number }
      logSessionEvent(p.id, 'exited', { exitCode: p.exitCode })
    }
  })
  headlessManager.on('client-message', (channel: string, payload: unknown) => {
    clientRegistry.broadcast(channel, payload, terminalScope(payload))
    if (channel === IPC.HEADLESS_EXIT) {
      const p = payload as { id: string; exitCode: number }
      logSessionEvent(p.id, 'exited', { exitCode: p.exitCode })
    }
  })
  scheduler.on('client-message', (channel: string, payload: unknown) => {
    clientRegistry.broadcast(channel, payload)
  })

  scriptRunnerEvents.on(IPC.SCRIPT_DATA, (payload) => {
    clientRegistry.broadcast(IPC.SCRIPT_DATA, payload)
  })
  scriptRunnerEvents.on(IPC.SCRIPT_EXIT, (payload) => {
    clientRegistry.broadcast(IPC.SCRIPT_EXIT, payload)
  })

  // ─── Persistent session auto-save ──────────────────────────────
  // Combined with explicit saves on key lifecycle events (session-created,
  // session-exit, SessionStart hook), this reduces reliance on the shutdown
  // path (which has a race with bridge.close and doesn't cover
  // force-quit / crash).
  //
  // Live sessions *and* the ones a previous run left unclaimed. A save is a
  // whole-table replace, so persisting only the live set is what erased every
  // record from the last run the moment a single pane was opened -- and with the
  // record gone, the next start judged that session's history unreachable and
  // deleted it. Holding them here is what makes a terminal survive more than one
  // restart.
  sessionManager.startAutoSave(() => [...ptyManager.getActiveSessions(), ...restoredRecords()])

  // ─── Hook server integration ──────────────────────────────────

  // Handle new terminal sessions: broadcast to UI + Copilot hook setup
  ptyManager.on('session-created', (session, payload) => {
    clientRegistry.broadcast(IPC.SESSION_CREATED, session)
    logSessionEvent(session.id, 'created', {
      agentType: session.agentType,
      projectName: session.projectName,
      projectPath: session.projectPath,
      ...(session.branch && { branch: session.branch })
    })

    if (payload.agentType === 'copilot') {
      const port = hookServer.getPort()
      if (port <= 0) return
      const cwd = session.worktreePath || session.projectPath
      const installation = installCopilotHooks(cwd, port)
      copilotInstallations.set(session.id, installation)
      hookStatusMapper.forceLink(installation.sessionId, session.id)
      session.hookSessionId = installation.sessionId
      // Don't set statusSource = 'hooks' eagerly — it disables the pattern-based
      // fallback. If hooks actually fire, promoteToHookStatus is called on the
      // first event. This fixes status stuck on 'waiting' when hooks don't work
      // (e.g. the agent CLI doesn't support hooks.json).
    }

    // For agents without session ID pinning (copilot, codex, opencode), read
    // the agent's own DB after it starts to capture the real session ID.
    // This enables reliable --resume on next app restart.
    if (
      supportsExactSessionResume(payload.agentType) &&
      !supportsSessionIdPinning(payload.agentType)
    ) {
      const captureSessionId = session.id
      // Asked more than once: an agent slow to write its own history used to be
      // read at five seconds, come up empty, and never be asked again -- leaving
      // the session holding a conversation it could not name, which a later
      // resume was then free to take.
      const attempt = (remaining: number[]): void => {
        const [delay, ...rest] = remaining
        if (delay === undefined) return
        setTimeout(() => {
          const s = ptyManager.getActiveSessions().find((t) => t.id === captureSessionId)
          if (!s) {
            releaseSpawningTranscriptsFor(captureSessionId)
            return
          }
          if (s.agentSessionId) return
          const cwd = s.worktreePath || s.projectPath
          const capturedId = captureAgentSessionId(s.agentType, cwd)
          if (!capturedId) return attempt(rest)
          s.agentSessionId = capturedId
          // Its own record names the conversation now, so the spawn claim is spent.
          releaseSpawningTranscriptsFor(captureSessionId)
          sessionManager.scheduleSave()
          clientRegistry.broadcast(IPC.SESSION_UPDATED, s)
          broadcastWidgetUpdate()
          log.info(`[session] captured ${s.agentType} session ID: ${capturedId}`)
        }, delay)
      }
      attempt([5000, 5000, 10_000, 20_000])
    }

    sessionManager.scheduleSave()
    broadcastWidgetUpdate()
  })

  // A shell moved. Saved on a debounce, so a script running `cd` in a loop costs
  // one write rather than hundreds -- and what is being kept is where the shell
  // ended up, not every step it took to get there.
  ptyManager.on('session-cwd', () => {
    sessionManager.scheduleSave()
  })

  // Clean up Copilot hooks on session exit
  ptyManager.on('session-exit', (session) => {
    // A session that died early holds nothing; without this its conversation
    // stays unreachable for the rest of the spawn window.
    releaseSpawningTranscriptsFor(session.id)
    const inst = copilotInstallations.get(session.id)
    if (inst) {
      uninstallCopilotHooks(inst)
      copilotInstallations.delete(session.id)
    }

    sessionManager.scheduleSave()
    broadcastWidgetUpdate()
  })

  // Start hook server
  hookServer
    .start()
    .then((port) => {
      try {
        // Only the instance that claimed the shared hook files writes the
        // settings entry that points at them. A dev server beside the packaged
        // app used to redirect its hooks here and, killed before it could tidy
        // up, leave them pointing at a port with no server behind it.
        if (hookServer.ownsRegistration()) {
          installHooks(port, hookServer.getAuthToken())
        } else {
          log.info('[hooks] another Vorn owns the registration; leaving it alone')
        }
      } catch (err) {
        log.error({ err }, '[hooks] failed to install hooks:')
      }

      hookServer.on('permission-cancelled', (requestId: string) => {
        clientRegistry.broadcast(IPC.WIDGET_PERMISSION_CANCELLED, requestId)
      })

      hookServer.on('hook-event', (event) => {
        log.info(`[hooks] ${event.hook_event_name}: session=${event.session_id} cwd=${event.cwd}`)
        const result = hookStatusMapper.mapEventToStatus(event)
        if (result) {
          ptyManager.updateSessionStatus(result.terminalId, result.status)
          ptyManager.promoteToHookStatus(result.terminalId)
          broadcastWidgetUpdate()

          // Persist after hookSessionId is set (SessionStart links the session)
          if (event.hook_event_name === 'SessionStart') {
            sessionManager.scheduleSave()
            try {
              const config = configManager.loadConfig()
              const task = config.tasks?.find(
                (t) =>
                  t.assignedSessionId === result.terminalId &&
                  t.status === 'in_progress' &&
                  !t.agentSessionId
              )
              if (task) {
                task.agentSessionId = event.session_id
                task.updatedAt = new Date().toISOString()
                configManager.saveConfig(config)
                log.info(
                  `[hooks] stored agentSessionId ${event.session_id} on task "${task.title}"`
                )
              }
            } catch (err) {
              log.error({ err }, '[hooks] failed to persist agentSessionId:')
            }
          }
        }

        const dismissEvents = ['PostToolUse', 'PostToolUseFailure', 'Stop', 'UserPromptSubmit']
        if (dismissEvents.includes(event.hook_event_name)) {
          hookServer.cancelSessionPermissions(event.session_id)
        }
      })

      hookServer.on('permission-request', ({ requestId, event }) => {
        const terminalId =
          hookStatusMapper.getLinkedTerminal(event.session_id) ??
          hookStatusMapper.tryLink(event.session_id, event.cwd)

        log.info(
          `[hooks] permission-request: session=${event.session_id} tool=${event.tool_name} → terminal=${terminalId ?? 'none (passthrough)'}`
        )

        if (!terminalId) {
          hookServer.passthroughPermission(requestId)
          return
        }

        ptyManager.promoteToHookStatus(terminalId)

        const session = ptyManager.getActiveSessions().find((s) => s.id === terminalId)

        const permReq: PermissionRequestInfo = {
          requestId,
          sessionId: event.session_id,
          terminalId,
          toolName: event.tool_name || 'unknown',
          toolInput: event.tool_input || {},
          description:
            typeof event.tool_input?.file_path === 'string'
              ? (event.tool_input.file_path as string)
              : typeof event.tool_input?.command === 'string'
                ? (event.tool_input.command as string)
                : typeof event.tool_input?.description === 'string'
                  ? (event.tool_input.description as string)
                  : undefined,
          agentType: session?.agentType,
          projectName: session?.projectName,
          permissionSuggestions: event.permission_suggestions,
          questions:
            event.tool_name === 'AskUserQuestion'
              ? (event.tool_input?.questions as PermissionRequestInfo['questions'] | undefined)
              : undefined
        }

        clientRegistry.broadcast(IPC.WIDGET_PERMISSION_REQUEST, permReq)
        ptyManager.updateSessionStatus(terminalId, 'waiting')
        broadcastWidgetUpdate()
      })
    })
    .catch((err) => {
      log.error('Failed to start hook server:', err)
    })
}

let widgetUpdateTimer: ReturnType<typeof setTimeout> | null = null

function broadcastWidgetUpdate(): void {
  if (widgetUpdateTimer) return
  widgetUpdateTimer = setTimeout(() => {
    widgetUpdateTimer = null
    const sessions = ptyManager.getActiveSessions()
    const agents: WidgetAgentInfo[] = sessions.map((s) => ({
      id: s.id,
      agentType: s.agentType,
      displayName: s.displayName,
      projectName: s.projectName,
      status: s.status
    }))
    clientRegistry.broadcast(IPC.WIDGET_STATUS_UPDATE, agents)
  }, 500)
}

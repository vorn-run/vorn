import crypto from 'node:crypto'
import { registerMethod, registerNotification } from './ws-handler'
import { ptyManager } from './pty-manager'
import { headlessManager } from './headless-manager'
import { configManager } from './config-manager'
import { sessionManager } from './session-persistence'
import { scheduler } from './scheduler'
import { scheduleLogManager } from './schedule-log'
import { getRecentSessions } from './agent-history'
import { detectIDEs, openInIDE } from './ide-detector'
import { detectInstalledAgents, clearAgentDetectionCache } from './agent-detector'
import { clientRegistry } from './broadcast'
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
  getProjectRemoteHostId
} from '@vornrun/shared/types'
import type { SourceConnection, TaskStatus } from '@vornrun/shared/types'
import * as gitUtils from './git-utils'
import { listDir, readFileContent, writeFileContent } from './file-utils'
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
  dbGetMaxTaskOrder,
  dbSignalChange,
  dbInsertWorkflow,
  dbDeleteWorkflow,
  dbGetWorkflow,
  dbListWorkflows
} from './database'
import {
  connectorRegistry,
  setDecryptedCreds,
  clearDecryptedCreds,
  applyDecryptedCreds,
  invokeMcpTool,
  discoverTools,
  mcpConnectionActions,
  stopMcpClient
} from './connectors'
import { MCP_CONNECTOR_ID } from './connectors/mcp'
import { detectRepoSlug } from './connectors/github'
import { buildConnectorSeededWorkflow } from './default-workflows'
import { connectorSeededWorkflowId, connectorSeededWorkflowIdPrefix } from '@vornrun/shared/types'
import { executeScript, scriptRunnerEvents } from './script-runner'
import { getTailscaleStatus, clearBinaryCache } from './tailscale'
import { checkAndRebind } from './server-rebind'
import { testSshConnection } from './process-utils'
import { captureAgentSessionId } from './agent-session-capture'
import { supportsExactSessionResume, supportsSessionIdPinning } from '@vornrun/shared/types'
import log from './logger'

const copilotInstallations = new Map<string, CopilotHookInstallation>()

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

  const orphan = dbFindTaskByConnectorExternalId(conn.connectorId, item.externalId)
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
    sourceConnectorId: conn.connectorId,
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
    log.error('[session-events] failed to log event:', err)
  }
}

let serverPort = 0
export function setServerPort(port: number): void {
  serverPort = port
}

export function registerAllMethods(): void {
  // Wire headless worktree counter into pty-manager for cleanup gating
  ptyManager.setHeadlessWorktreeCounter((worktreePath, excludeId) =>
    headlessManager.getActiveSessionsForWorktree(worktreePath, excludeId)
  )

  // Terminal
  registerMethod('terminal:create', (payload) => {
    return ptyManager.createPty(payload)
  })
  registerMethod('terminal:kill', (id) => ptyManager.killPty(id))
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
  registerMethod('sessions:getPrevious', () => sessionManager.getPreviousSessions())
  registerMethod('sessions:clear', () => sessionManager.clear())
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
  registerMethod('git:removeWorktree', ({ projectPath, worktreePath, force }) => {
    const remote = resolveRemoteHost(projectPath)
    return gitUtils.removeWorktree(projectPath, worktreePath, force, remote)
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
  registerMethod('ide:open', ({ ideId, projectPath }) => openInIDE(ideId, projectPath))

  // Tailscale network access
  registerMethod('tailscale:status', async () => {
    clearBinaryCache() // Always re-detect in case user just installed
    await checkAndRebind() // Rebind if Tailscale state changed since startup
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

  // File explorer
  registerMethod('file:listDir', ({ dirPath, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return listDir(dirPath, remote)
  })
  registerMethod('file:readContent', ({ filePath, maxBytes, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return readFileContent(filePath, maxBytes, remote)
  })
  registerMethod('file:writeContent', ({ filePath, content, remoteHostId }) => {
    const remote = remoteHostId ? resolveRemoteHostById(remoteHostId) : undefined
    return writeFileContent(filePath, content, remote)
  })

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
      status: 'success' | 'error'
      sessionsLaunched: number
      source?: 'scheduler' | 'manual'
    }) => {
      if (data.status !== 'success' && data.status !== 'error') return
      if (data.source === 'scheduler') {
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
    return dbListSourceConnections(connectorId)
  })

  registerMethod('connection:create', (params) => {
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

    // Seed visible + editable default workflows from the connector manifest.
    const connector = connectorRegistry.get(conn.connectorId)
    if (connector) {
      const manifest = connector.describe()
      for (const event of manifest.defaultWorkflows ?? []) {
        const wfId = connectorSeededWorkflowId(conn.id, event.event)
        if (dbGetWorkflow(wfId)) continue
        const wf = buildConnectorSeededWorkflow(conn, manifest, event)
        dbInsertWorkflow(wf)
        log.info(`[connector] seeded workflow ${wfId} for connection ${conn.id}`)
      }
    }

    dbSignalChange()
    configManager.notifyChanged()

    // For MCP connections, kick off tool discovery in the background. We
    // delay briefly so the main process has time to decrypt + push secretEnv
    // via the credential-sync path (triggered by notifyChanged above).
    if (conn.connectorId === MCP_CONNECTOR_ID) {
      setTimeout(() => {
        void runMcpDiscovery(conn.id).catch((err) =>
          log.warn(`[mcp] initial discovery failed for ${conn.id}: ${err}`)
        )
      }, 1500)
    }

    return conn
  })

  registerMethod('connection:update', ({ id, updates }) => {
    dbUpdateSourceConnection(id, updates)
    dbSignalChange()
    return dbGetSourceConnection(id)
  })

  registerMethod('connection:delete', (id) => {
    // Delete any seeded workflows tied to this connection. User-created
    // workflows that reference this connection stay — deleting a connection
    // should never silently remove a workflow the user built by hand.
    const prefix = connectorSeededWorkflowIdPrefix(id)
    for (const wf of dbListWorkflows()) {
      if (wf.id.startsWith(prefix)) {
        dbDeleteWorkflow(wf.id)
      }
    }
    // task_source_links cascade via FK. Tasks themselves stay and retain
    // their source_connector_id / source_external_id metadata — so a later
    // connection add + backfill can re-adopt them via the orphan dedup path
    // in upsertExternalItem instead of creating duplicates.
    dbDeleteSourceConnection(id)
    // Forget any decrypted plaintext for this connection.
    clearDecryptedCreds(id)
    // Terminate any live MCP stdio child for this connection. Fire-and-forget
    // because delete is synchronous and the child may take a moment to exit.
    void stopMcpClient(id).catch((err) => log.warn(`[mcp] stopClient failed: ${err}`))
    dbSignalChange()
    configManager.notifyChanged()
  })

  registerMethod('workflow:runManual', ({ workflowId }) => {
    scheduler.triggerWorkflow(workflowId)
  })

  registerMethod('credentials:setDecrypted', ({ connectionId, fields }) => {
    setDecryptedCreds(connectionId, fields)
  })

  registerMethod('credentials:clearDecrypted', ({ connectionId }) => {
    clearDecryptedCreds(connectionId)
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
    const tools = conn.filters.discoveredTools
    return Array.isArray(tools) ? tools : []
  })

  registerMethod('connection:refreshMcpTools', async (connectionId: string) => {
    return runMcpDiscovery(connectionId)
  })

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
    if (!connector?.listItems) {
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
      const items = await connector.listItems(applyDecryptedCreds(conn))
      for (const item of items) {
        const initialStatus = conn.statusMapping?.[item.status] || ('todo' as TaskStatus)
        const result = upsertExternalItem(
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
        if (result.created) imported++
        else updated++
      }
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
    for (const c of connectorRegistry.list()) {
      if (c.id === 'github') {
        const { resolveGhPath, ghInstallHint, getGhEnv } = await import('./connectors/gh-cli')
        const ghPath = resolveGhPath()
        if (!ghPath) {
          results.push({
            connectorId: c.id,
            authed: false,
            message: `GitHub CLI (gh) is not installed or not on PATH.\n${ghInstallHint()}\nAfter installing, run: \`gh auth login\``
          })
          continue
        }
        try {
          const { execFile } = await import('node:child_process')
          const { promisify } = await import('node:util')
          const execFileAsync = promisify(execFile)
          await execFileAsync(ghPath, ['auth', 'status'], {
            timeout: 5_000,
            env: getGhEnv()
          })
          results.push({ connectorId: c.id, authed: true })
        } catch (err) {
          // gh is installed but the status probe failed. The common case is
          // "not signed in"; anything else we surface in logs so it's not
          // silently lost.
          const msg = err instanceof Error ? err.message : String(err)
          log.warn(`[connector:status] gh auth status failed: ${msg}`)
          results.push({
            connectorId: c.id,
            authed: false,
            message: 'Sign in by running `gh auth login` in your terminal.'
          })
        }
      } else {
        results.push({ connectorId: c.id, authed: true })
      }
    }
    return results
  })

  // Wire manager events → broadcast to WS clients
  ptyManager.on('client-message', (channel: string, payload: unknown) => {
    clientRegistry.broadcast(channel, payload)
    if (channel === IPC.TERMINAL_EXIT) {
      const p = payload as { id: string; exitCode: number }
      logSessionEvent(p.id, 'exited', { exitCode: p.exitCode })
    }
  })
  headlessManager.on('client-message', (channel: string, payload: unknown) => {
    clientRegistry.broadcast(channel, payload)
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
  sessionManager.startAutoSave(() => ptyManager.getActiveSessions())

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
      setTimeout(() => {
        const s = ptyManager.getActiveSessions().find((t) => t.id === captureSessionId)
        if (!s || s.agentSessionId) return
        const cwd = s.worktreePath || s.projectPath
        const capturedId = captureAgentSessionId(s.agentType, cwd)
        if (capturedId) {
          s.agentSessionId = capturedId
          sessionManager.scheduleSave()
          clientRegistry.broadcast(IPC.SESSION_UPDATED, s)
          broadcastWidgetUpdate()
          log.info(`[session] captured ${s.agentType} session ID: ${capturedId}`)
        }
      }, 5000)
    }

    sessionManager.scheduleSave()
    broadcastWidgetUpdate()
  })

  // Clean up Copilot hooks on session exit
  ptyManager.on('session-exit', (session) => {
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
        installHooks(port, hookServer.getAuthToken())
      } catch (err) {
        log.error('[hooks] failed to install hooks:', err)
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
              log.error('[hooks] failed to persist agentSessionId:', err)
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

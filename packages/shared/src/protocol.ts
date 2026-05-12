import type {
  CreateTerminalPayload,
  TerminalSession,
  HeadlessSession,
  AppConfig,
  ResizePayload,
  FileEntry,
  GitDiffStat,
  GitDiffResult,
  WorkflowExecution,
  ScriptConfig,
  ScheduleLogEntry,
  RecentSession,
  PermissionRequestInfo,
  WidgetAgentInfo,
  WorkflowDefinition,
  SSHKey,
  SSHKeyMeta,
  SessionEvent,
  SessionEventType,
  SourceConnection,
  TaskSourceLink,
  ConnectorManifest,
  ConnectorItemContext,
  TaskStatus
} from './types'

// ─── JSON-RPC 2.0 Envelope Types ────────────────────────────────

export interface RpcRequest {
  jsonrpc: '2.0'
  id: number | string
  method: string
  params?: unknown
}

export interface RpcResponse {
  jsonrpc: '2.0'
  id: number | string
  result?: unknown
  error?: RpcError
}

export interface RpcError {
  code: number
  message: string
  data?: unknown
}

export interface RpcNotification {
  jsonrpc: '2.0'
  method: string
  params?: unknown
}

// ─── Request Methods (client → server, invoke-style) ────────────

export interface RequestMethods {
  'terminal:create': { params: CreateTerminalPayload; result: TerminalSession }
  'terminal:kill': { params: string; result: void }
  'terminal:listActive': { params: void; result: TerminalSession[] }
  'terminal:rename': { params: { id: string; displayName: string }; result: void }
  'terminal:reorder': { params: string[]; result: void }
  'terminal:readOutput': { params: { id: string; lines?: number }; result: string[] }
  'shell:create': { params: string | undefined; result: TerminalSession }
  'config:load': { params: void; result: AppConfig }
  'config:save': { params: AppConfig; result: void }
  'sessions:getPrevious': { params: void; result: TerminalSession[] }
  'sessions:clear': { params: void; result: void }
  'sessions:getRecent': { params: string | undefined; result: RecentSession[] }
  'git:isGitRepo': { params: string; result: boolean }
  'git:listBranches': {
    params: string
    result: { local: string[]; current: string | null; isGitRepo: boolean }
  }
  'git:listRemoteBranches': { params: string; result: string[] }
  'git:createWorktree': {
    params: { projectPath: string; branch: string }
    result: string
  }
  'git:removeWorktree': {
    params: { projectPath: string; worktreePath: string; force?: boolean }
    result: void
  }
  'git:renameWorktreeBranch': {
    params: { worktreePath: string; newBranch: string }
    result: boolean
  }
  'git:worktreeDirty': { params: string; result: boolean }
  'git:listWorktrees': {
    params: string
    result: Array<{ path: string; branch: string; isBare: boolean }>
  }
  'git:diffStat': { params: string; result: GitDiffStat }
  'git:diffFull': { params: string; result: GitDiffResult }
  'git:commit': {
    params: { cwd: string; message: string; includeUnstaged: boolean }
    result: { success: boolean; error?: string }
  }
  'git:push': { params: string; result: { success: boolean; error?: string } }
  'scheduler:getLog': {
    params: string | undefined
    result: ScheduleLogEntry[]
  }
  'scheduler:getNextRun': { params: string; result: string | null }
  'task:imageSave': {
    params: { taskId: string; sourcePath: string }
    result: string
  }
  'task:imageDelete': {
    params: { taskId: string; filename: string }
    result: void
  }
  'task:imageGetPath': {
    params: { taskId: string; filename: string }
    result: string
  }
  'task:imageCleanup': { params: string; result: void }
  'headless:create': {
    params: CreateTerminalPayload
    result: HeadlessSession
  }
  'headless:kill': { params: string; result: void }
  'headless:list': { params: void; result: HeadlessSession[] }
  'script:execute': { params: ScriptConfig; result: { output: string; exitCode: number } }
  'workflowRun:save': { params: WorkflowExecution; result: void }
  'workflowRun:list': {
    params: { workflowId: string; limit?: number }
    result: WorkflowExecution[]
  }
  'workflowRun:listByTask': {
    params: { taskId: string; limit?: number }
    result: WorkflowExecution[]
  }
  'sessionEvent:list': {
    params: { eventType?: SessionEventType; limit?: number }
    result: SessionEvent[]
  }
  'sessionEvent:listBySession': {
    params: { sessionId: string; limit?: number }
    result: SessionEvent[]
  }
  'worktree:activeSessions': {
    params: string
    result: { count: number; sessionIds: string[] }
  }
  'agent:detectInstalled': {
    params: void
    result: Record<string, boolean>
  }
  'ide:detect': { params: void; result: Array<{ id: string; name: string }> }
  'ide:open': {
    params: { ideId: string; projectPath: string }
    result: void
  }
  'permission:resolve': {
    params: {
      requestId: string
      allow: boolean
      updatedPermissions?: unknown[]
      updatedInput?: unknown
    }
    result: void
  }
  'server:shutdown': { params: void; result: void }

  // Credential vault (server-side storage)
  'credential:storeKey': {
    params: {
      label: string
      encryptedPrivateKey: string
      publicKey?: string
      certificate?: string
      keyType?: string
    }
    result: { id: string }
  }
  'credential:listKeys': { params: void; result: SSHKeyMeta[] }
  'credential:deleteKey': { params: string; result: void }
  'credential:getEncryptedKey': { params: string; result: SSHKey | null }

  // File explorer
  'file:listDir': { params: { dirPath: string; remoteHostId?: string }; result: FileEntry[] }
  'file:readContent': {
    params: { filePath: string; maxBytes?: number; remoteHostId?: string }
    result: string | null
  }
  'file:writeContent': {
    params: { filePath: string; content: string; remoteHostId?: string }
    result: { success: boolean; error?: string }
  }

  // Connectors
  'connector:list': {
    params: void
    result: Array<{
      id: string
      name: string
      icon: string
      capabilities: string[]
      manifest: ConnectorManifest
    }>
  }
  'connector:get': {
    params: string
    result: {
      id: string
      name: string
      icon: string
      capabilities: string[]
      manifest: ConnectorManifest
    } | null
  }
  'connection:list': {
    params: { connectorId?: string }
    result: SourceConnection[]
  }
  'connection:create': {
    params: Omit<
      SourceConnection,
      'id' | 'createdAt' | 'lastSyncAt' | 'lastSyncError' | 'syncCursor'
    >
    result: SourceConnection
  }
  'connection:update': {
    params: { id: string; updates: Partial<SourceConnection> }
    result: SourceConnection | null
  }
  'connection:delete': {
    params: string
    result: void
  }
  /** Trigger a workflow manually via the scheduler — same dispatch path as
   *  cron, so connectorPoll triggers do their full poll+fan-out. */
  'workflow:runManual': {
    params: { workflowId: string }
    result: void
  }
  /** Main→server push of decrypted credential fields. Called after main
   *  decrypts values (via Electron safeStorage) on boot and on config
   *  changes. Plaintext lives in server memory only — never persisted. */
  'credentials:setDecrypted': {
    params: { connectionId: string; fields: Record<string, string> }
    result: void
  }
  /** Clear the in-memory plaintext for a connection (on delete / sign-out). */
  'credentials:clearDecrypted': {
    params: { connectionId: string }
    result: void
  }
  /** Invoke a connector's action (createIssue, commentOnIssue, ...) via the
   *  connection's auth. Used by callConnectorAction workflow nodes. */
  'connection:executeAction': {
    params: {
      connectionId: string
      action: string
      args: Record<string, unknown>
    }
    result: { success: boolean; output?: Record<string, unknown>; error?: string }
  }
  /** One-shot backfill of existing items for a connection — bypasses the
   *  "since" cursor that poll() uses, calling listItems() directly. Respects
   *  the connection's filters. Used by the "Import existing" button. */
  'connection:backfill': {
    params: { connectionId: string }
    result: { imported: number; updated: number; error?: string }
  }
  /** Upsert a single external item into the task board. Called by the
   *  `createTaskFromItem` workflow node for each fan-out from a connector poll. */
  'connection:upsertFromItem': {
    params: {
      connectionId: string
      item: ConnectorItemContext
      /** Initial status for a NEW task; never overwrites local status on re-sync. */
      initialStatus: TaskStatus
      /** Project name override; `undefined` defers to the connection's executionProject. */
      project?: string
    }
    result: { taskId: string; created: boolean }
  }
  'connection:getSourceLink': {
    params: string
    result: TaskSourceLink | null
  }
  'connector:detectRepo': {
    params: string
    result: { owner: string; repo: string } | null
  }
  /** Seed (or re-seed) the default workflow for a (connection × event). Idempotent. */
  'connector:seedWorkflow': {
    params: { connectionId: string; event: string }
    result: { workflowId: string; created: boolean }
  }
  /** Report connector auth/health status — e.g. whether `gh` is signed in. */
  'connector:status': {
    params: void
    result: Array<{ connectorId: string; authed: boolean; message?: string }>
  }
}

// ─── Server Notifications (server → client, push events) ────────

export interface ServerNotifications {
  'terminal:data': { id: string; data: string }
  'terminal:exit': { id: string; exitCode: number }
  'session:created': TerminalSession
  'session:updated': TerminalSession
  'session:reordered': string[]
  'headless:data': { id: string; data: string }
  'headless:exit': { id: string; exitCode: number }
  'config:changed': AppConfig
  'widget:status-update': WidgetAgentInfo[]
  'widget:permission-request': PermissionRequestInfo
  'widget:permission-cancelled': string
  'worktree:confirmCleanup': {
    terminalId: string
    worktreePath: string
    projectPath: string
    branch?: string
  }
  'scheduler:execute': {
    workflowId: string
    workflow: WorkflowDefinition
    /** Populated when the scheduler fan-outs a connector-poll result. One
     *  scheduler:execute is emitted per new item, each carrying its own item
     *  context. Consumed by createTaskFromItem nodes (and any downstream
     *  nodes that reference context.connectorItem). */
    connectorItem?: ConnectorItemContext
  }
  'scheduler:missed': Array<{
    workflowId: string
    workflowName: string
    missedAt: string
  }>
  'workflow:executionComplete': WorkflowExecution
  'session-exit': TerminalSession
  'database:corruption-recovered': { message: string }
}

// ─── Client Notifications (client → server, fire-and-forget) ────

export interface ClientNotifications {
  'terminal:write': { id: string; data: string }
  'terminal:resize': ResizePayload
}

// ─── Typed helpers ──────────────────────────────────────────────

export type RequestMethod = keyof RequestMethods
export type ServerNotification = keyof ServerNotifications
export type ClientNotification = keyof ClientNotifications

export function createRequest<M extends RequestMethod>(
  id: number,
  method: M,
  params: RequestMethods[M]['params']
): RpcRequest {
  return { jsonrpc: '2.0', id, method, params }
}

export function createNotification(method: string, params?: unknown): RpcNotification {
  return { jsonrpc: '2.0', method, params }
}

export function createResponse(id: number | string, result: unknown): RpcResponse {
  return { jsonrpc: '2.0', id, result }
}

export function createErrorResponse(
  id: number | string,
  code: number,
  message: string,
  data?: unknown
): RpcResponse {
  return { jsonrpc: '2.0', id, error: { code, message, data } }
}

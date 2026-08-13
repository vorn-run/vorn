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
  SSHKey,
  SSHKeyMeta,
  SessionEvent,
  SessionEventType,
  SourceConnection,
  TaskSourceLink,
  ConnectorManifest,
  ConnectorItemContext,
  TaskStatus,
  WorktreeInventory,
  WorktreeActionResult,
  BranchDeleteResult,
  BrowserPageRead,
  BrowserConsoleMessage,
  BrowserNetworkRequest,
  BrowserNode,
  BrowserTarget,
  DeviceInfo,
  DeviceElement,
  DeviceScreenRead,
  DeviceTarget,
  DevicePoint,
  MobileProject
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
    params: {
      projectPath: string
      worktreePath: string
      force?: boolean
      /** Delete the worktree's branch too, when it is safe to (merged, or forced). */
      deleteBranch?: boolean
    }
    /** False when `git worktree remove` refused — callers surface it as an error. */
    result: boolean
  }
  'worktree:inventory': {
    params: { projectPaths?: string[]; refresh?: boolean } | undefined
    result: WorktreeInventory
  }
  'worktree:reclaimArtifacts': {
    params: { paths: string[] }
    result: WorktreeActionResult
  }
  'worktree:removeMany': {
    params: {
      items: Array<{
        projectPath: string
        worktreePath: string
        force?: boolean
        deleteBranch?: boolean
      }>
    }
    result: WorktreeActionResult
  }
  'worktree:pruneOrphans': {
    params: { paths: string[] }
    result: WorktreeActionResult
  }
  'git:deleteBranches': {
    params: { projectPath: string; branches: string[]; force?: boolean }
    result: BranchDeleteResult
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
  /**
   * Whether a project looks like a mobile app, so the device control can be
   * offered where it is useful and withheld where it would be noise.
   *
   * Advisory only: the device tools themselves are never gated on this. An
   * agent asked to open a simulator for a project this misreads must still be
   * able to, or a wrong guess becomes a dead end instead of a missing button.
   */
  'project:detectMobile': {
    params: { projectPath: string }
    result: MobileProject
  }
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

  // Intent bar completions
  'shell:listExecutables': { params: void; result: string[] }

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
    > & {
      /**
       * Polling workflow to seed alongside the connection.
       *
       * Built-ins declare this on their manifest, which the server reads
       * directly. A packaged connector cannot: the connector the registry
       * resolves for it is the generic MCP one, so what the probe read from
       * the package is passed through here instead.
       */
      seedWorkflow?: { name: string; defaultCronFromMinutes: number }
    }
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
    params: { workflowId: string; inputs?: Record<string, unknown> }
    result: void
  }
  /** Ask a packaged connection whether it could run right now. `ok: null`
   *  means the connector declares no preflight — nothing to check, which is
   *  not the same answer as "checked, fine". */
  'connection:preflight': {
    params: string
    result: { ok: boolean | null; message?: string }
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
  /** A renderer accepted a durable connector event and finished its workflow.
   * Failures are retried with bounded exponential backoff. */
  'connector:inboxComplete': {
    params: {
      id: number
      leaseToken: string
      disposition: 'processed' | 'retry' | 'defer'
      error?: string
    }
    result: void
  }
  'connector:inboxRenew': {
    params: { id: number; leaseToken: string }
    result: boolean
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

  // ─── Agent-controllable browser pane ──────────────────────────
  //
  // These are the one family of methods the server does not itself implement.
  // A `<webview>` guest is only reachable from the Electron main process, so
  // the server forwards each call back over the same bridge main uses to reach
  // it (see `browserBridge` in packages/server) and main answers from its CDP
  // registry. Every one is session-scoped by the *caller's* identity —
  // `VORN_SESSION_ID`, resolved in the MCP layer — and none of them accepts a
  // session argument, so one session cannot address another's pane.
  'browser:readPage': {
    params: { sessionId: string; filter?: 'interactive' | 'all'; cursor?: string; limit?: number }
    result: BrowserPageRead
  }
  'browser:getText': {
    params: { sessionId: string; cursor?: string }
    result: { url: string; text: string; nextCursor?: string }
  }
  'browser:consoleMessages': {
    params: { sessionId: string; limit?: number }
    result: BrowserConsoleMessage[]
  }
  'browser:networkRequests': {
    params: { sessionId: string; limit?: number }
    result: BrowserNetworkRequest[]
  }
  'browser:screenshot': {
    params: { sessionId: string; fullPage?: boolean }
    /** Base64 PNG. Deliberately the last resort — every other read is cheaper. */
    result: { data: string }
  }
  'browser:interact': {
    params: {
      sessionId: string
      action: 'click' | 'hover' | 'type' | 'key' | 'scroll'
      target?: BrowserTarget
      /** Text for `type`, key name for `key`, pixel delta for `scroll`. */
      text?: string
      deltaY?: number
    }
    result: { ok: true }
  }
  'browser:tabs': {
    params: { sessionId: string; action: 'add' | 'close' | 'select'; url?: string; index?: number }
    result: { ok: true }
  }
  'browser:openPane': {
    params: { sessionId: string; url?: string }
    result: { url: string }
  }
  'browser:navigate': {
    params: { sessionId: string; url: string }
    result: { url: string }
  }
  'browser:find': {
    params: { sessionId: string; text: string; limit?: number }
    result: BrowserNode[]
  }

  // ─── Device (iOS simulator) ───────────────────────────────────
  //
  // Forwarded to main over the same bridge and for a sharper version of the
  // same reason: a simulator is driven by a child `idb_companion` process
  // speaking gRPC over a unix socket, which only main owns.
  //
  // Every method here carries `sessionId` explicitly. That is the opposite of
  // the MCP tool surface, where no tool takes a session argument because the
  // MCP layer resolves it from `VORN_SESSION_ID` before calling in. By the
  // time a request reaches this contract the resolution has already happened,
  // so callers must pass the id rather than expect it to be inferred.
  //
  // `device:list` is the one method that works without a claim; it is how a
  // session discovers what there is to claim without leaving Vorn.
  'device:list': {
    params: { sessionId: string }
    result: DeviceInfo[]
  }
  'device:claim': {
    params: { sessionId: string; udid: string }
    result: { udid: string; name: string; booted: boolean }
  }
  'device:release': {
    params: { sessionId: string }
    result: { released: boolean }
  }
  'device:readScreen': {
    params: { sessionId: string; filter?: 'interactive' | 'all'; cursor?: string; limit?: number }
    result: DeviceScreenRead
  }
  'device:find': {
    params: { sessionId: string; query: string; limit?: number }
    result: { elements: DeviceElement[]; generation: number }
  }
  'device:interact': {
    params: {
      sessionId: string
      action: 'tap' | 'swipe' | 'type' | 'button' | 'press'
      target?: DeviceTarget
      /** Swipe destination, in points. */
      to?: DevicePoint
      /** Text for `type`; button name for `button` (HOME, LOCK, SIRI…). */
      text?: string
      /** Seconds to hold, for a long press. */
      duration?: number
      /**
       * Opt in to a stroke starting inside the bezel band, which iOS claims as
       * a system gesture. Refused by default: swallowed silently, it reads to
       * the agent as a swipe that did nothing.
       */
      systemGesture?: boolean
    }
    result: { ok: true; generation: number }
  }
  'device:screenshot': {
    params: { sessionId: string; maxEdge?: number }
    /** Base64 PNG, downscaled in main. `scale` converts image pixels back to
     *  the points every ref and tap is expressed in. */
    result: { data: string; scale: number; screen: { width: number; height: number } }
  }
  'device:launch': {
    params: { sessionId: string; bundleId: string }
    result: { ok: true }
  }
  'device:terminate': {
    params: { sessionId: string; bundleId: string }
    result: { ok: true }
  }
  'device:install': {
    params: { sessionId: string; path: string }
    result: { ok: true }
  }
  'device:openUrl': {
    params: { sessionId: string; url: string }
    result: { ok: true }
  }
  'device:logs': {
    params: { sessionId: string; limit?: number }
    result: { lines: string[] }
  }
  'device:openPane': {
    params: { sessionId: string; udid?: string }
    result: { udid: string }
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
    inputs?: Record<string, unknown>
    /** Populated when the scheduler fan-outs a connector-poll result. One
     *  scheduler:execute is emitted per new item, each carrying its own item
     *  context. Consumed by createTaskFromItem nodes (and any downstream
     *  nodes that reference context.connectorItem). */
    connectorItem?: ConnectorItemContext
    connectorInboxId?: number
    connectorInboxLeaseToken?: string
    existingExecution?: WorkflowExecution
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

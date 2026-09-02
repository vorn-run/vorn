import type {
  CreateTerminalPayload,
  RestoredSession,
  TerminalSession,
  HeadlessSession,
  AppConfig,
  ResizePayload,
  FileEntry,
  GitDiffStat,
  GitDiffResult,
  WorkflowDefinition,
  WorkflowExecution,
  ScriptConfig,
  ScheduleLogEntry,
  ProjectConfig,
  RecentSession,
  PermissionRequestInfo,
  WidgetAgentInfo,
  SSHKey,
  SSHKeyMeta,
  SessionEvent,
  SessionEventType,
  SourceConnection,
  TaskSourceLink,
  ConnectorKey,
  ConnectorManifest,
  ConnectorItemContext,
  TaskConfig,
  TaskStatus,
  AiAgentType,
  WorktreeInventory,
  WorktreeActionResult,
  BranchDeleteResult,
  BrowserPageRead,
  BrowserConsoleMessage,
  BrowserNetworkRequest,
  BrowserNode,
  BrowserTabInfo,
  BrowserTarget,
  DeviceInfo,
  DeviceElement,
  DeviceScreenRead,
  DeviceTarget,
  DevicePoint,
  MobileProject,
  ReachableUrls,
  PairingRequest,
  DeviceToken,
  ConnectorActionDef,
  ConnectorCatalogSnapshot,
  ConnectorInstallProgress,
  ConnectorPackPreview,
  ConnectorPackResult,
  ConnectorPackSource,
  InstalledConnectorPack,
  SdkProbeRequest,
  SdkProbeResult,
  InstalledShell,
  TailscaleStatus,
  RemoteHost
} from './types'

// ─── Runtime Protocol Version ───────────────────────────────────

/**
 * The wire contract between a client and this server.
 *
 * Once clients update on their own schedule — a phone, a browser, a desktop
 * pointed at someone else's host — mixed versions are the normal state rather
 * than an edge case, and negotiation cannot be added after the fact: every
 * client that predates it is already unable to negotiate. So the handshake
 * ships while there is exactly one client and it costs nothing.
 *
 * Bump this when an existing message changes shape or meaning. A new *optional*
 * field does not need a bump, but stays safe only while every reader treats it
 * as optional — the moment one requires it, that reader is broken against every
 * older server, which is the same defect as removing a field, found later.
 */
export const RUNTIME_PROTOCOL_VERSION = 1

export interface ServerHello {
  protocolVersion: number
  /**
   * What this server can do, as name → version. A client sends a new message
   * kind only after seeing it here, because `ws-handler` drops unknown methods
   * silently: an unnegotiated feature appears to hang rather than to fail.
   *
   * `auth: 1` means the server refuses every method until a credential is
   * presented — either as `Authorization: Bearer` on the upgrade, or as an
   * `auth:authenticate` message.
   */
  capabilities: Record<string, number>
}

/**
 * Who this server is, for a desktop deciding whether to adopt it rather than
 * start its own.
 *
 * A separate frame from `server:hello` because the audience is different, and
 * that difference is the whole design. Capabilities go to every client;
 * identity goes only to a peer on loopback, because `dataDir` names the user's
 * home directory and the server may be bound to `0.0.0.0`. Folding it into the
 * greeting meant one message with two audiences — a frame that had to be built
 * per-peer and cached twice, and fields that had to be optional for readers who
 * would never receive them.
 *
 * Every field is required, so a reader that has this frame is done checking. A
 * server too old to send one simply does not, and "no identity" is already the
 * answer that declines adoption.
 *
 * Adoption turns on `ServerHello.protocolVersion`, never on `appVersion`. The
 * two move independently on purpose — the protocol changes when the messages
 * change, the release changes every time anything ships — and gating on the
 * release would end every running session on every update for no reason. The
 * same reasoning is why a mismatch is never settled by killing the incumbent:
 * the server holding the PTYs holds the user's work, so a client that cannot
 * speak to it declines and says so.
 */
export interface ServerIdentity {
  appVersion: string
  /** Resolved data directory. Two servers on one directory is the case to catch. */
  dataDir: string
  pid: number
  /**
   * A dev build and a packaged build deliberately share `~/.vorn` while keeping
   * separate Electron user data, so without this a `yarn dev` launch would adopt
   * the packaged app's bundled server, or the reverse.
   */
  buildChannel: 'dev' | 'packaged'
  /**
   * How many terminals this server currently has a process behind.
   *
   * Optional, so a server that predates it still validates. It rides this frame
   * rather than an RPC because a launcher decides whether to adopt *before* it
   * authenticates and closes the socket the moment it refuses — so for five of
   * the six refusal reasons there is no request it could make. This is the only
   * thing it hears from a server it has just declined.
   *
   * Treat it as a number from an untrusted party: coerce and clamp it, never
   * splice it into a sentence.
   */
  sessions?: number
}

// ─── Authentication ─────────────────────────────────────────────

/**
 * Close codes. Both are in the private-use range (4000–4999) reserved for
 * applications, so they cannot collide with a protocol-level close.
 *
 * They are distinct because clients act on them differently: a rejected
 * credential is worth discarding, a timeout is not. Conflating them means a
 * backgrounded phone whose socket stalled throws away a perfectly good token.
 */
export const CLOSE_UNAUTHENTICATED = 4001
export const CLOSE_CREDENTIAL_REJECTED = 4002

/** JSON-RPC error code for a method sent before authenticating. */
export const RPC_NOT_AUTHENTICATED = -32001

/**
 * The environment variable the desktop passes its per-launch credential in.
 *
 * Named here because three packages depend on the exact string: the desktop
 * sets it, the server reads it, and `process-utils` strips it so it can never
 * reach a PTY. A rename that missed any one of those would be silent.
 */
export const BOOTSTRAP_ENV_VAR = 'SECRET_VORN_BOOTSTRAP_TOKEN'

/**
 * The port a Vorn server takes when it has no reason to take another.
 *
 * Vorn had no default at all until now: the port was whatever the OS handed the
 * first launch, remembered and reused. That only ever reached step two -- see
 * `resolveServerPort` -- so in practice every launch drew a new one, and the
 * origin a browser keys its token by changed underneath it every time.
 *
 * An install that already remembers a port keeps it; this decides a first run.
 * The number itself is not special. It is the one existing installs happen to
 * hold, which is worth more than a prettier constant that would move them.
 */
export const DEFAULT_SERVER_PORT = 50091

/**
 * The environment variable that overrides the port for one launch.
 *
 * Named here because the desktop launcher reads it and the server is what it
 * ends up configuring. It exists for the case a stored setting cannot serve: a
 * dev server and a packaged Vorn share one data directory, so they share one
 * remembered port, and the whole point is for them to differ.
 */
export const SERVER_PORT_ENV_VAR = 'VORN_SERVER_PORT'

/** Filename, under the resolved data dir, of the credential same-machine tools read. */
export const LOCAL_TOKEN_FILENAME = 'local-token'

/**
 * Filename, under the resolved data dir, of the running server's `{port, pid}`.
 *
 * Named here because three packages now depend on the exact string: the server
 * writes it, `packages/mcp` reads it to find a server it did not start, and the
 * desktop launcher reads it to decide whether to adopt one instead of spawning a
 * second. It was a bare literal in the first two until the third arrived.
 */
export const WS_PORT_FILENAME = 'ws-port'

/**
 * Filename, under the resolved data dir, of the canonical local endpoint.
 *
 * A unix socket rather than a record naming a port, because this one is *owned*:
 * `link()` refuses to replace an existing name and `rename()` replaces one
 * atomically, so a starting server can ask whether the machine already has a
 * server before it becomes a second one. A port has no such name -- two servers
 * reaching for one are settled by whichever bound first, invisibly.
 *
 * Absent on win32, where Node maps a socket path to a named pipe and there is no
 * filesystem entry to test-and-set. There the port file above remains the only
 * answer, and the race it leaves open remains open.
 */
export const ENDPOINT_FILENAME = 'vorn.sock'

/**
 * Exit code for a server that found this machine already had one.
 *
 * Distinct from both success and failure, because it is neither. The process did
 * nothing wrong and nothing is broken -- it simply arrived second, and the right
 * answer is for the app that started it to adopt the incumbent instead. Read as
 * a crash it would be relaunched, and the relaunch would arrive second too.
 */
export const EXIT_ENDPOINT_TAKEN = 3

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
  /** Present a credential. The only method accepted before authenticating. */
  'auth:authenticate': { params: { token: string }; result: { ok: boolean } }
  'server:reachableUrls': { params: void; result: ReachableUrls }
  'webhook:info': { params: void; result: { baseUrl: string } }
  // Device tokens. Namespaced `token:` rather than `device:`, which belongs
  // entirely to the simulator registry — thirteen methods of it.
  'token:list': { params: void; result: DeviceToken[] }
  'token:create': { params: { name: string }; result: { token: DeviceToken; plaintext: string } }
  'token:revoke': { params: string; result: { revoked: boolean } }
  // Pairing a phone by showing it a code. The phone's own two calls are HTTP,
  // not here: it has no credential yet, and the socket admits exactly one
  // method before authenticating.
  'pairing:start': { params: void; result: { code: string; expiresAt: number } }
  'pairing:approve': { params: { requestId: string }; result: { ok: boolean } }
  'pairing:deny': { params: { requestId: string }; result: { ok: boolean } }
  'pairing:cancel': { params: void; result: { ok: boolean } }
  'pairing:pending': { params: void; result: PairingRequest[] }

  // ── Declared late ────────────────────────────────────────────────
  //
  // These have had live handlers and live callers for a long time while being
  // absent from this map, so `registerMethod` fell back to `unknown` and every
  // one of them was untyped end to end — a renamed field or a changed shape would
  // have been caught by nothing. The signatures below are taken from the handlers
  // as they actually behave, not from what they arguably should.

  // Git
  'git:getBranch': { params: string; result: string | null }
  'git:getWorktreeBranch': { params: string; result: string | null }
  'git:checkoutBranch': {
    params: { cwd: string; branch: string }
    result: { ok: boolean; error?: string }
  }
  'git:renameWorktree': {
    params: { worktreePath: string; newName: string }
    result: { newPath: string; name: string } | null
  }

  // Connections and connectors
  'connection:listActions': { params: string; result: ConnectorActionDef[] }
  'connection:listMcpTools': {
    params: string
    result: Array<{ name: string; description?: string; inputSchema?: Record<string, unknown> }>
  }
  'connection:refreshMcpTools': {
    params: string
    result: { ok: boolean; count?: number; error?: string }
  }
  'connector:catalog': { params: void; result: ConnectorCatalogSnapshot }
  'connector:catalogRefresh': { params: void; result: ConnectorCatalogSnapshot }
  'connector:probeSdk': { params: SdkProbeRequest; result: SdkProbeResult }
  'connector:inspectPack': { params: ConnectorPackSource; result: ConnectorPackPreview }
  'connector:installPack': { params: ConnectorPackSource; result: ConnectorPackResult }
  'connector:removePack': {
    params: string
    result: { ok: boolean; error?: string; connections?: number }
  }
  'connector:rollbackPack': { params: string; result: ConnectorPackResult }
  'connector:listPacks': { params: void; result: InstalledConnectorPack[] }

  // Workflow runs
  'workflowRun:claim': {
    params: { workflowId: string; params?: string; windowMs?: number }
    result: { granted: boolean; runId: string }
  }
  'workflowRun:release': {
    params: { workflowId: string; params?: string; runId: string }
    result: void
  }
  'workflowRun:listRunning': { params: void; result: WorkflowExecution[] }
  'workflowRun:listWaiting': { params: void; result: WorkflowExecution[] }
  'workflowRun:listAll': {
    params: { workspaceId?: string; limit?: number }
    result: (WorkflowExecution & { workflowName?: string })[]
  }
  'workflow:executionComplete': {
    params: {
      workflowId: string
      workflowName: string
      completedAt: string
      status: 'success' | 'error' | 'cancelled'
      sessionsLaunched: number
      source?: 'scheduler' | 'manual'
    }
    result: void
  }

  // Host and shell
  'shell:listInstalled': { params: void; result: InstalledShell[] }
  'ssh:testConnection': {
    params: RemoteHost
    result: { success: boolean; message: string; durationMs: number }
  }
  'tailscale:status': { params: void; result: TailscaleStatus }

  // Task images and UI
  /**
   * The task board on its own, rather than as part of the whole configuration.
   *
   * Tasks live in `AppConfig.tasks`, so reading the board has meant loading the
   * entire config and changing one status has meant sending all of it back. On
   * a real board that is around 95 KB of a 104 KB payload, which is invisible on
   * a desk and the difference between usable and not on a phone.
   */
  'task:list': {
    params: { projectName?: string; status?: TaskStatus; includeDescription?: boolean } | void
    result: TaskConfig[]
  }
  'task:setStatus': { params: { id: string; status: TaskStatus }; result: { ok: boolean } }

  /**
   * Writing a task, rather than only reading and moving one.
   *
   * These reach the same row-level functions the board has always had. Without
   * them the only way to write a task over this socket is `config:save` with the
   * whole configuration attached — which is the round trip `task:list` above
   * exists to avoid, paid on every keystroke's worth of change.
   *
   * `ok: false` means there was nothing to write to: an id that named no task,
   * or, for `task:create`, a project that does not exist. `task:create` returns
   * the row so the caller does not have to guess the id it was given or the
   * order it landed at.
   */
  'task:create': {
    params: {
      projectName: string
      title: string
      description?: string
      status?: TaskStatus
      branch?: string
      useWorktree?: boolean
      assignedAgent?: AiAgentType
    }
    result: { ok: boolean; task?: TaskConfig }
  }
  'task:update': {
    params: {
      id: string
      /** Moving a task between projects. It lands at the end of the new one. */
      projectName?: string
      title?: string
      description?: string
      status?: TaskStatus
      branch?: string
      useWorktree?: boolean
      assignedAgent?: AiAgentType
    }
    result: { ok: boolean; task?: TaskConfig }
  }
  'task:delete': { params: { id: string }; result: { ok: boolean } }
  /**
   * The ids in the order they should now sit. Anything absent keeps its place.
   *
   * There is no project to name because the named tasks are permuted through
   * the places they already hold: ids drawn from two projects would each stay
   * among the orders their own rows already had. One project at a time is the
   * ordinary use, not a rule the server enforces.
   */
  'task:reorder': { params: { ids: string[] }; result: { ok: boolean } }
  /**
   * One method rather than two. Archiving and restoring are the same field
   * being set and cleared; the MCP side has two tools only because a tool is a
   * verb.
   */
  'task:archive': { params: { id: string; archived: boolean }; result: { ok: boolean } }

  /**
   * The projects a session can be launched into.
   *
   * Small, and inside the configuration with everything else — so asking for it
   * alone costs a few hundred bytes rather than the hundred kilobytes that
   * carrying the task board along with it does.
   */
  'project:list': { params: void; result: ProjectConfig[] }

  /** One task, description included. The listing omits descriptions. */
  'task:get': { params: { id: string }; result: TaskConfig | null }

  /**
   * One workflow's definition.
   *
   * A run records node ids and their outcome, not what the nodes are called, so
   * anything drawing a trace needs the definition beside it. `config:load`
   * carries every workflow, and on a phone that is a hundred kilobytes to label
   * a handful of rows
   */
  'workflow:get': { params: { id: string }; result: WorkflowDefinition | null }

  /**
   * Every workflow, whole.
   *
   * `workflow:get` needs an id, and until this there was no method that produced
   * one — so a client that had not already been told about a workflow could not
   * reach it at all. The phone's Workflows tab is a list of *runs* for exactly
   * that reason, and a workflow that has never run appears nowhere in it.
   *
   * The definition is not trimmed, unlike `task:list`, which blanks descriptions.
   * That looked like the same problem and is not: measured against a real board,
   * task descriptions are 147 KB while every workflow on the same machine —
   * nodes, edges, agent prompts and all — is 5.4 KB. There is nothing here worth
   * the cost of a second shape that has to be kept in step with this one.
   *
   * No workspace filter. `workflowRun:listAll` takes one, but nothing that would
   * call this has a workspace to pass yet; it is a line to add when a caller
   * exists rather than an argument nobody sends.
   */
  'workflow:list': { params: void; result: WorkflowDefinition[] }

  /**
   * Switch a workflow's schedule on or off.
   *
   * `enabled` governs whether a trigger fires, not whether a run may be started
   * by hand, and it is meaningless on a manual workflow — there is nothing to
   * disable. The desktop writes it through the whole-config save path; this is
   * the same change as one call.
   */
  'workflow:setEnabled': {
    params: { id: string; enabled: boolean }
    result: { ok: boolean }
  }

  /**
   * Answer a run that is parked on an approval gate.
   *
   * The decision is recorded by whichever instance is holding the run, not here:
   * resuming means re-entering the execution engine, and that lives in a desktop
   * window rather than in this process. So this broadcasts and returns, the same
   * way stopping a run does — every instance hears it and only the owner acts.
   *
   * `accepted` says the message went out, not that a run resumed. If no desktop
   * is open there is nothing to act on it, and the gate stays open until one is.
   */
  'workflow:resolveGate': {
    params: { runId: string; nodeId: string; decision: 'approve' | 'reject' }
    result: { accepted: boolean }
  }

  'task:imageUpload': {
    params: { taskId: string; base64: string; filename: string }
    result: string
  }
  'permission:resolve-top': { params: { allow: boolean }; result: void }
  'widget:requestUpdate': { params: void; result: void }
  'terminal:create': { params: CreateTerminalPayload; result: TerminalSession }
  'terminal:kill': { params: string; result: void }
  'terminal:listActive': { params: void; result: TerminalSession[] }
  'terminal:rename': { params: { id: string; displayName: string }; result: void }
  'terminal:reorder': { params: string[]; result: void }
  'terminal:readOutput': { params: { id: string; lines?: number }; result: string[] }
  /**
   * The terminal's output as it was emitted, escape sequences intact.
   *
   * `terminal:readOutput` is the same output with every sequence removed, which
   * is right for reading and useless for drawing. A client attaching a terminal
   * emulator feeds this in first, then applies live `terminal:data`; without it
   * the screen stays blank until the program next repaints, which for an idle
   * agent may be never.
   */
  'terminal:readScrollback': { params: { id: string }; result: { data: string } }
  /**
   * Everything a pane needs to start showing a terminal it did not create.
   *
   * `data` is the scrollback; `seq` is the flush it reflects, so the caller can
   * discard the live chunks it already has; `live` says whether a process is
   * still behind it, which decides whether the pane is a terminal or a
   * photograph of one. All three are read in the same tick, and that is what
   * makes the pair trustworthy -- see `PtyManager.flushSeq`.
   *
   * It serves a session restored from disk as well as a running one. There is
   * no process behind a restored session, so nothing can arrive while the answer
   * is in flight and `seq` is zero.
   */
  'terminal:attach': {
    params: { id: string }
    result: { data: string; seq: number; live: boolean }
  }
  'shell:create': { params: string | undefined; result: TerminalSession }
  'config:load': { params: void; result: AppConfig }
  'config:save': { params: AppConfig; result: void }
  /**
   * Sessions from the last run that no pane has taken yet.
   *
   * The server holds them, so two clients can be looking at the same one. Each
   * says roughly when it ended and whether there is a screen to show, which is
   * what a pane needs to say what it is looking at rather than pretending it is
   * live.
   */
  'sessions:restored': { params: void; result: RestoredSession[] }
  /**
   * Claim one and start it, exactly once.
   *
   * `gone` is the honest answer to the second caller: another pane, another
   * window or another device took it first, and starting a second agent against
   * one transcript is the failure this prevents.
   */
  'sessions:resume': {
    params: { id: string }
    result: /** `boundTo` names the session already writing this conversation, when one was. */
      | { ok: true; session: TerminalSession; boundTo?: string }
      | { ok: false; reason: 'gone' | 'failed'; message?: string }
  }
  'sessions:clear': { params: void; result: void }
  'sessions:getRecent': { params: string | undefined; result: RecentSession[] }
  'git:isGitRepo': { params: string; result: boolean }
  'git:listBranches': {
    params: string
    result: { local: string[]; current: string | null; isGitRepo: boolean }
  }
  'git:listRemoteBranches': { params: string; result: string[] }
  'git:createWorktree': {
    params: { projectPath: string; branch: string; worktreeName?: string }
    result: { worktreePath: string; branch: string; name: string }
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
    result: Array<{ path: string; branch: string; isMain: boolean; name: string }>
  }
  'git:diffStat': { params: string; result: GitDiffStat | null }
  'git:diffFull': { params: string; result: GitDiffResult | null }
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
  'script:execute': {
    params: ScriptConfig
    result: { success: boolean; output: string; error?: string; exitCode?: number }
  }
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
  /** Ask whichever instance owns this run to stop it. Broadcast, not
   *  addressed: runs execute in the renderer, so the server cannot end one
   *  itself and does not know which window holds it. */
  'workflow:stopRun': {
    params: { runId: string }
    result: void
  }
  /** Ask a packaged connection whether it could run right now. `ok: null`
   *  means the connector declares no preflight — nothing to check, which is
   *  not the same answer as "checked, fine". Throws for a connection that does
   *  not exist, so a stale id cannot read as "nothing to check". */
  'connection:preflight': {
    params: string
    result: { ok: boolean | null; message?: string }
  }
  /** Every connection holding a secret, described without disclosing one. */
  'connection:listKeys': {
    params: void
    result: ConnectorKey[]
  }
  /** Replace one stored secret. `value` is the ciphertext to persist, sealed
   *  by the desktop process because only it holds the keychain. `plaintext` is
   *  the same secret in the clear, handed over so the running server can serve
   *  it immediately rather than leaving a window in which the key is stored
   *  but unusable — the same trust boundary `credentials:setDecrypted` uses. */
  'connection:rotateSecret': {
    params: { connectionId: string; field: string; value: string; plaintext: string }
    result: { ok: boolean; error?: string }
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
  /** Execute one HTTP request server-side, applying an optional profile's
   *  auth injection there so secrets never reach the renderer. */
  'http:request': {
    params: {
      profileConnectionId?: string
      method: string
      url: string
      headers?: Record<string, string>
      body?: string
    }
    result: { success: boolean; output?: Record<string, unknown>; error?: string }
  }
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
  /**
   * Ask the tool a connector borrows whether it is signed in, and as whom.
   * `ok: null` means the rung has nothing this probe can answer.
   */
  'connector:probeAuth': {
    params: string
    result: { ok: boolean | null; identity?: string; message?: string; installHint?: string }
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
  'browser:listTabs': {
    params: { sessionId: string }
    result: { tabs: BrowserTabInfo[] }
  }
  'browser:history': {
    params: { sessionId: string; direction: 'back' | 'forward' }
    /** Where the pane landed. */
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
  /** First frame on every connection, before anything is dispatched. */
  'server:hello': ServerHello
  /** Follows the greeting, on loopback only. Absent from an older server. */
  'server:identity': ServerIdentity
  /** Sent once a socket is admitted, so a client knows it may start sending. */
  'auth:ok': { userId: string }
  /**
   * `seq` numbers the flush this chunk came from, counting up per session.
   *
   * It exists so a pane can attach without losing or repeating anything.
   * `terminal:attach` answers with the scrollback *and* the flush it reflects;
   * a client subscribes first, buffers, and then applies only the chunks
   * numbered above what it was handed.
   */
  'terminal:data': { id: string; data: string; seq: number }
  'terminal:exit': { id: string; exitCode: number }
  'session:created': TerminalSession
  'session:updated': TerminalSession
  'session:reordered': string[]
  'headless:data': { id: string; data: string }
  'headless:exit': { id: string; exitCode: number }
  'config:changed': AppConfig
  /** How far a connector pack install has got, while it runs. */
  'connector:installProgress': ConnectorInstallProgress
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
  /** A person answered a gate. Only the instance holding the run acts on it. */
  'workflow:gateResolved': { runId: string; nodeId: string; decision: 'approve' | 'reject' }
  'session-exit': TerminalSession
  /** A phone offered a valid pairing code and is waiting to be approved. */
  'pairing:requested': PairingRequest
  /**
   * A phone collected the token it was approved for, which is the moment the
   * credential starts existing. Approving does not mint one: a request nobody
   * collects should leave nothing behind.
   */
  'pairing:collected': { requestId: string }
  'database:corruption-recovered': { message: string }
}

// ─── Client Notifications (client → server, fire-and-forget) ────

export interface ClientNotifications {
  'terminal:write': { id: string; data: string }
  'terminal:resize': ResizePayload
  /**
   * Narrow which server notifications this socket receives.
   *
   * An entry is an exact name (`config:changed`) or a namespace wildcard
   * (`session:*`). Omitting `topics` restores everything, which is also the
   * default, so a client that never sends this is unaffected. An empty list
   * does the same rather than silencing the socket: there is no way to ask for
   * nothing, because a client that wants nothing can close, whereas one that
   * sent `[]` by accident would look broken with no signal saying why.
   *
   * Only honoured when `server:hello` advertised the `subscribe` capability.
   * Prefer the `topics` query parameter on the socket URL for the initial set:
   * this message can only take effect after the socket is already receiving.
   */
  'subscribe:set': { topics?: readonly string[] }
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

import { CLOSE_CREDENTIAL_REJECTED, RUNTIME_PROTOCOL_VERSION } from '@vornrun/shared/protocol'
import { captureViewerSettings, withViewerSettings } from '@vornrun/shared/viewer-settings-store'
import type { AppConfig } from '@vornrun/shared/types'
/**
 * WebSocket RPC shim that implements the same surface as the Electron preload `window.api`.
 * Components and stores call window.api.* exactly as they do in Electron,
 * but here each call is translated to a JSON-RPC 2.0 message over WebSocket.
 */

// ─── JSON-RPC Transport ─────────────────────────────────────────

type PendingRequest = {
  resolve: (result: unknown) => void
  reject: (error: Error) => void
}

const TOKEN_STORAGE_KEY = 'vorn.deviceToken'

export class AuthRequiredError extends Error {
  constructor() {
    super('A device token is required to connect to this Vorn server.')
    this.name = 'AuthRequiredError'
  }
}

/**
 * A call that depends on the machine running the server, refused where it cannot work.
 *
 * Rejecting rather than resolving with a plausible-looking empty value: these are
 * writes and device actions, and a caller that believes one succeeded is worse off
 * than one that sees why it did not.
 */
function unsupportedInWeb(what: string): () => Promise<never> {
  return () =>
    Promise.reject(
      new Error(`${what} is only available in the Vorn app on the machine running the server.`)
    )
}

export function readStoredToken(): string {
  try {
    return localStorage.getItem(TOKEN_STORAGE_KEY) ?? ''
  } catch {
    return ''
  }
}

export function storeToken(token: string): void {
  try {
    localStorage.setItem(TOKEN_STORAGE_KEY, token.trim())
  } catch {
    /* private browsing — the user will be asked again next load */
  }
}

function clearStoredToken(): void {
  try {
    localStorage.removeItem(TOKEN_STORAGE_KEY)
  } catch {
    /* nothing to clear */
  }
}

class RpcClient {
  private ws!: WebSocket
  private nextId = 1
  private pending = new Map<number, PendingRequest>()
  private listeners = new Map<string, Set<(params: unknown) => void>>()
  private url: string
  private _ready!: Promise<void>
  private _resolveReady!: () => void
  private _rejectReady!: (err: Error) => void
  /** Whether `_ready` has settled, so `resetReady` knows if replacing it is safe. */
  private readySettled = false
  /**
   * Whether there is one yet at all.
   *
   * The constructor's own call is the only moment there is not, and `_ready` is
   * declared with a definite-assignment assertion -- so testing the promise for
   * truthiness read as always true and the guard below never fired on the case it
   * was written for.
   */
  private hasReady = false
  /**
   * Called when the server rejects our credential, on any connection.
   *
   * A callback rather than only the readiness promise: `_ready` is replaced on
   * every reconnect, and the bootstrap only ever observes the first one — so
   * rejecting it after a reconnect rejects a promise nobody is watching, and the
   * page silently stops updating instead of asking for a token.
   */
  private onAuthRequired: (() => void) | null = null
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null

  constructor(url: string) {
    this.url = url
    this.resetReady()
    this.connect()
  }

  /**
   * A fresh readiness promise per connection attempt — but only once the current
   * one has settled.
   *
   * `main.tsx` awaits `__ready()` exactly once, at startup, and renders when it
   * resolves. Replacing an unsettled promise orphans that await: the retry can
   * connect and authenticate perfectly, resolving the *new* promise, while the one
   * the app is holding stays pending forever and the loading screen never lifts.
   * That is easy to hit — the first attempt loses whenever the page opens a moment
   * before the server is listening.
   *
   * So a pending promise is kept and allowed to settle on a later attempt, and only
   * a settled one is replaced, which is what `invoke()` needs so a call made after a
   * drop waits for the next connection rather than resolving against the closed one.
   */
  private resetReady(): void {
    if (this.hasReady && !this.readySettled) return
    this.hasReady = true
    this.readySettled = false
    this._ready = new Promise((resolve, reject) => {
      this._resolveReady = () => {
        this.readySettled = true
        resolve()
      }
      this._rejectReady = (err) => {
        this.readySettled = true
        reject(err)
      }
    })
    // Nothing awaits a replacement promise, so an unobserved rejection would
    // surface as an unhandled rejection in the console.
    this._ready.catch(() => {})
  }

  setOnAuthRequired(handler: () => void): void {
    this.onAuthRequired = handler
  }

  private onVersionMismatch: ((server: number, client: number) => void) | null = null

  setOnVersionMismatch(handler: (server: number, client: number) => void): void {
    this.onVersionMismatch = handler
  }

  private connect(): void {
    this.ws = new WebSocket(this.url)

    this.ws.onopen = () => {
      // A browser cannot set headers on the upgrade, so the credential goes in
      // the first message. Nothing else is accepted until the server has it, and
      // `_ready` stays pending until it does — so no caller can send a request
      // that would be refused.
      this.ws.send(
        JSON.stringify({
          jsonrpc: '2.0',
          method: 'auth:authenticate',
          params: { token: readStoredToken() }
        })
      )
      if (this.reconnectTimer) {
        clearTimeout(this.reconnectTimer)
        this.reconnectTimer = null
      }
    }

    this.ws.onmessage = (event) => {
      let msg: {
        id?: number
        method?: string
        result?: unknown
        error?: { message: string }
        params?: unknown
      }
      try {
        msg = JSON.parse(event.data as string)
      } catch {
        return
      }

      // The handshake, sent before anything else. Compared rather than ignored so
      // a mismatch says so plainly: an old cached bundle against a newer server
      // otherwise fails later, in ways that read as the app being broken.
      if (msg.method === 'server:hello') {
        const serverVersion = (msg.params as { protocolVersion?: number } | undefined)
          ?.protocolVersion
        if (typeof serverVersion === 'number' && serverVersion !== RUNTIME_PROTOCOL_VERSION) {
          this.onVersionMismatch?.(serverVersion, RUNTIME_PROTOCOL_VERSION)
        }
        return
      }

      // The server confirming our credential. Only now is the connection usable,
      // so this — not `onopen` — is what settles `_ready`.
      if (msg.method === 'auth:ok') {
        this._resolveReady()
        return
      }

      // Server push notification (no id)
      if (msg.method && msg.id === undefined) {
        const cbs = this.listeners.get(msg.method)
        if (cbs) {
          for (const cb of cbs) {
            try {
              cb(msg.params)
            } catch {
              /* ignore listener errors */
            }
          }
        }
        return
      }

      // Response to a pending request
      if (msg.id !== undefined) {
        const pending = this.pending.get(msg.id)
        if (pending) {
          this.pending.delete(msg.id)
          if (msg.error) {
            pending.reject(new Error(msg.error.message))
          } else {
            pending.resolve(msg.result)
          }
        }
      }
    }

    this.ws.onclose = (event) => {
      // Reject all pending requests
      for (const [, p] of this.pending) {
        p.reject(new Error('WebSocket disconnected'))
      }
      this.pending.clear()

      // The server rejected the credential. Retrying cannot help, and the old
      // behaviour — reconnect every 2s forever while `__ready()` never settled —
      // showed a blank page rather than a reason.
      //
      // Only on a rejection, never on CLOSE_UNAUTHENTICATED: that also covers an
      // auth timeout, and discarding a good token because a backgrounded phone's
      // socket stalled would send the user back to the machine running Vorn.
      if (event.code === CLOSE_CREDENTIAL_REJECTED) {
        clearStoredToken()
        // Rejects the promise for a first load, and fires the callback for a
        // reconnect — where the promise the bootstrap is awaiting has long since
        // resolved and a rejection would go unobserved.
        this._rejectReady(new AuthRequiredError())
        this.onAuthRequired?.()
        return
      }

      // Auto-reconnect after 2s
      this.reconnectTimer = setTimeout(() => {
        this.resetReady()
        this.connect()
      }, 2000)
    }

    this.ws.onerror = () => {
      // onclose will fire after this
    }
  }

  ready(): Promise<void> {
    return this._ready
  }

  /** Request-response RPC call */
  invoke(method: string, params?: unknown): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const id = this.nextId++
      this.pending.set(id, { resolve, reject })
      const msg = JSON.stringify({ jsonrpc: '2.0', id, method, params })
      if (this.ws.readyState === WebSocket.OPEN) {
        this.ws.send(msg)
      } else {
        // Capture the current ready promise to detect replacement on close
        const readyAtCall = this._ready
        readyAtCall.then(() => {
          if (this.ws.readyState === WebSocket.OPEN) {
            this.ws.send(msg)
          } else if (this.pending.has(id)) {
            this.pending.delete(id)
            reject(new Error('WebSocket not connected'))
          }
        })
      }
    })
  }

  /** Fire-and-forget notification (no response expected) */
  notify(method: string, params?: unknown): void {
    const msg = JSON.stringify({ jsonrpc: '2.0', method, params })
    if (this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(msg)
    }
  }

  /** Subscribe to server push notifications. Returns unsubscribe function. */
  on(event: string, callback: (params: unknown) => void): () => void {
    let cbs = this.listeners.get(event)
    if (!cbs) {
      cbs = new Set()
      this.listeners.set(event, cbs)
    }
    cbs.add(callback)
    return () => {
      cbs!.delete(callback)
      if (cbs!.size === 0) {
        this.listeners.delete(event)
      }
    }
  }
}

// ─── File Picker Helpers ─────────────────────────────────────────

function pickFiles(accept: string, multiple = false): Promise<File[] | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = accept
    input.multiple = multiple
    input.style.display = 'none'
    document.body.appendChild(input)

    let resolved = false
    const cleanup = () => {
      if (input.parentNode) input.parentNode.removeChild(input)
    }

    input.addEventListener('change', () => {
      resolved = true
      const files = input.files ? Array.from(input.files) : null
      cleanup()
      resolve(files && files.length > 0 ? files : null)
    })

    // Handle cancel: 'cancel' event + focus fallback for older browsers
    input.addEventListener('cancel', () => {
      if (!resolved) {
        resolved = true
        cleanup()
        resolve(null)
      }
    })

    // Fallback: detect cancel via window focus return (for Safari/older browsers)
    const onFocus = () => {
      setTimeout(() => {
        if (!resolved) {
          resolved = true
          cleanup()
          resolve(null)
        }
        window.removeEventListener('focus', onFocus)
      }, 300)
    }
    window.addEventListener('focus', onFocus)

    input.click()
  })
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const dataUrl = reader.result as string
      // Strip "data:...;base64," prefix
      resolve(dataUrl.split(',')[1])
    }
    reader.onerror = reject
    reader.readAsDataURL(file)
  })
}

// ─── API Shim ───────────────────────────────────────────────────

export function createApiShim(wsUrl: string) {
  const rpc = new RpcClient(wsUrl)

  const api = {
    // ── Ready (web-only, not in Electron API) ──
    __ready: () => rpc.ready(),
    /** Fires whenever the server rejects our credential, including on a reconnect. */
    __onAuthRequired: (handler: () => void) => rpc.setOnAuthRequired(handler),
    __onVersionMismatch: (handler: (server: number, client: number) => void) =>
      rpc.setOnVersionMismatch(handler),

    // ── Terminal Management ──
    createTerminal: (payload: unknown) => rpc.invoke('terminal:create', payload),
    writeTerminal: (id: string, data: string) => rpc.notify('terminal:write', { id, data }),
    resizeTerminal: (payload: unknown) => rpc.notify('terminal:resize', payload),
    killTerminal: (id: string) => rpc.invoke('terminal:kill', id),
    createShellTerminal: (cwd?: string) => rpc.invoke('shell:create', cwd),

    // ── Terminal Events ──
    onTerminalData: (callback: (event: { id: string; data: string; seq: number }) => void) =>
      rpc.on('terminal:data', callback as (p: unknown) => void),
    onTerminalExit: (callback: (event: { id: string; exitCode: number }) => void) =>
      rpc.on('terminal:exit', callback as (p: unknown) => void),
    onSessionCreated: (callback: (session: unknown) => void) =>
      rpc.on('session:created', callback as (p: unknown) => void),

    // ── Configuration ──
    // See the note in src/preload/index.ts — the same two hooks, so a browser and a
    // desktop pointed at one server each keep their own view.
    loadConfig: async () => withViewerSettings((await rpc.invoke('config:load')) as AppConfig),
    saveConfig: (config: unknown) => {
      captureViewerSettings(config as AppConfig)
      return rpc.invoke('config:save', config)
    },
    onConfigChanged: (callback: (config: unknown) => void) =>
      rpc.on('config:changed', (p: unknown) => callback(withViewerSettings(p as AppConfig))),

    // ── Menu Events (Electron-only, no-op in web) ──
    // Desktop only. A browser or phone connects to a server; it never launches
    // one, so there is no local server for it to be looking away from.
    onLocalServerStillRunning: (_callback: (notice: unknown) => void) => () => {},
    stopLocalServer: async () => ({ ok: false, error: 'Not available here.' }),
    // Present so the surface test passes; the palette hides the command in web,
    // so nothing should reach this.
    stopSessionsAndServer: unsupportedInWeb('Stopping the server'),

    onMenuNewAgent: (_callback: () => void) => () => {},

    // ── Sessions ──
    listActiveSessions: () => rpc.invoke('terminal:listActive'),
    attachTerminal: (id: string) => rpc.invoke('terminal:attach', { id }),
    getRestoredSessions: () => rpc.invoke('sessions:restored'),
    // The desktop hears this from its own launcher, which is the thing that
    // notices a server has died and starts another. A web client has no
    // launcher: it reconnects to whatever answers the address it was given, and
    // has no way to be told that what answered is a different process. Present
    // so the surface matches, and so a caller does not have to know which client
    // it is running in.
    onServerReplaced: (_callback: () => void) => () => {},
    resumeSession: (params: { id: string }) => rpc.invoke('sessions:resume', params),
    clearPreviousSessions: () => rpc.invoke('sessions:clear'),
    getRecentSessions: (projectPath?: string) => rpc.invoke('sessions:getRecent', projectPath),
    renameSession: (id: string, displayName: string) =>
      rpc.invoke('terminal:rename', { id, displayName }),
    reorderSessions: (ids: string[]) => rpc.invoke('terminal:reorder', ids),

    // ── Dialogs (web: use HTML5 file inputs) ──
    openDirectoryDialog: async (): Promise<string | null> => {
      // Web cannot pick server-side directories; prompt user to type a path
      const path = window.prompt('Enter the project directory path on the server:')
      return path?.trim() || null
    },
    openFileDialog: async (): Promise<string | null> => {
      // Web cannot pick server-side files; prompt user to type a path
      const path = window.prompt('Enter the file path on the server:')
      return path?.trim() || null
    },
    openImageDialog: async (): Promise<string[] | null> => {
      const files = await pickFiles('image/*', true)
      if (!files)
        return null
        // Store files for the next saveTaskImage call
      ;(api as Record<string, unknown>).__pendingImageFiles = files
      // Return fake paths so the caller iterates correctly
      return files.map((f) => f.name)
    },

    // ── IDE Detection & Launch ──
    detectIDEs: () => rpc.invoke('ide:detect'),
    detectInstalledAgents: () => rpc.invoke('agent:detectInstalled'),
    openInIDE: (ideId: string, projectPath: string) =>
      rpc.invoke('ide:open', { ideId, projectPath }),

    // ── Git Operations ──
    listBranches: (projectPath: string) => rpc.invoke('git:listBranches', projectPath),
    listRemoteBranches: (projectPath: string) => rpc.invoke('git:listRemoteBranches', projectPath),
    createWorktree: (projectPath: string, branch: string) =>
      rpc.invoke('git:createWorktree', { projectPath, branch }),
    removeWorktree: (
      projectPath: string,
      worktreePath: string,
      force?: boolean,
      deleteBranch?: boolean
    ) => rpc.invoke('git:removeWorktree', { projectPath, worktreePath, force, deleteBranch }),
    renameWorktreeBranch: (worktreePath: string, newBranch: string) =>
      rpc.invoke('git:renameWorktreeBranch', { worktreePath, newBranch }),
    isWorktreeDirty: (worktreePath: string) => rpc.invoke('git:worktreeDirty', worktreePath),
    listWorktrees: (projectPath: string) => rpc.invoke('git:listWorktrees', projectPath),
    getWorktreeActiveSessions: (worktreePath: string) =>
      rpc.invoke('worktree:activeSessions', worktreePath),
    getWorktreeInventory: (params?: { projectPaths?: string[]; refresh?: boolean }) =>
      rpc.invoke('worktree:inventory', params),
    reclaimWorktreeArtifacts: (paths: string[]) =>
      rpc.invoke('worktree:reclaimArtifacts', { paths }),
    removeWorktrees: (
      items: {
        projectPath: string
        worktreePath: string
        force?: boolean
        deleteBranch?: boolean
      }[]
    ) => rpc.invoke('worktree:removeMany', { items }),
    pruneOrphanWorktrees: (paths: string[]) => rpc.invoke('worktree:pruneOrphans', { paths }),
    deleteBranches: (projectPath: string, branches: string[], force?: boolean) =>
      rpc.invoke('git:deleteBranches', { projectPath, branches, force }),
    getGitDiffStat: (cwd: string) => rpc.invoke('git:diffStat', cwd),
    getGitDiffFull: (cwd: string) => rpc.invoke('git:diffFull', cwd),
    gitCommit: (payload: unknown) => rpc.invoke('git:commit', payload),
    gitPush: (cwd: string) => rpc.invoke('git:push', cwd),

    // ── Task Images (web: upload via base64 RPC, serve via HTTP) ──
    saveTaskImage: async (taskId: string, sourcePath: string): Promise<string> => {
      // In web mode, sourcePath is actually the original filename from openImageDialog
      const pendingFiles = (api as Record<string, unknown>).__pendingImageFiles as
        | File[]
        | undefined
      if (pendingFiles) {
        const file = pendingFiles.find((f) => f.name === sourcePath)
        if (file) {
          const base64 = await fileToBase64(file)
          const filename = (await rpc.invoke('task:imageUpload', {
            taskId,
            base64,
            filename: file.name
          })) as string
          // Remove uploaded file from pending list; clear when empty
          const remaining = pendingFiles.filter((f) => f.name !== sourcePath)
          ;(api as Record<string, unknown>).__pendingImageFiles =
            remaining.length > 0 ? remaining : undefined
          return filename
        }
      }
      // Fallback to server-side path (e.g. drag-and-drop won't work in web)
      return rpc.invoke('task:imageSave', { taskId, sourcePath }) as Promise<string>
    },
    deleteTaskImage: (taskId: string, filename: string) =>
      rpc.invoke('task:imageDelete', { taskId, filename }),
    getTaskImagePath: async (taskId: string, filename: string): Promise<string> => {
      // Return HTTP URL instead of filesystem path
      return `/api/task-images/${taskId}/${filename}`
    },
    cleanupTaskImages: (taskId: string) => rpc.invoke('task:imageCleanup', taskId),

    // ── Headless Sessions ──
    createHeadlessSession: (payload: unknown) => rpc.invoke('headless:create', payload),
    killHeadlessSession: (id: string) => rpc.invoke('headless:kill', id),
    onHeadlessData: (callback: (event: { id: string; data: string }) => void) =>
      rpc.on('headless:data', callback as (p: unknown) => void),
    onHeadlessExit: (callback: (event: { id: string; exitCode: number }) => void) =>
      rpc.on('headless:exit', callback as (p: unknown) => void),

    // ── Script Execution ──
    executeScript: (config: unknown) => rpc.invoke('script:execute', config),
    onScriptData: (callback: (event: { runId: string; data: string }) => void) =>
      rpc.on('script:data', callback as (p: unknown) => void),
    onScriptExit: (callback: (event: { runId: string; exitCode: number }) => void) =>
      rpc.on('script:exit', callback as (p: unknown) => void),

    // ── Worktree Cleanup ──
    onSessionUpdated: (callback: (session: unknown) => void) =>
      rpc.on('session:updated', callback as (p: unknown) => void),
    onWorktreeCleanup: (
      callback: (session: { id: string; projectPath: string; worktreePath: string }) => void
    ) => rpc.on('worktree:confirmCleanup', callback as (p: unknown) => void),

    // ── Scheduler ──
    getScheduleLog: (workflowId?: string) => rpc.invoke('scheduler:getLog', workflowId),
    getScheduleNextRun: (workflowId: string) => rpc.invoke('scheduler:getNextRun', workflowId),
    onSchedulerExecute: (
      callback: (event: {
        workflowId: string
        connectorItem?: import('../../shared/src/types').ConnectorItemContext
        connectorInboxId?: number
        connectorInboxLeaseToken?: string
        inputs?: Record<string, unknown>
        existingExecution?: import('../../shared/src/types').WorkflowExecution
      }) => void
    ) => rpc.on('scheduler:execute', callback as (p: unknown) => void),
    onSchedulerStopRun: (callback: (event: { runId: string }) => void) =>
      rpc.on('scheduler:stopRun', callback as (p: unknown) => void),
    onWorkflowGateResolved: (
      callback: (event: { runId: string; nodeId: string; decision: 'approve' | 'reject' }) => void
    ) => rpc.on('workflow:gateResolved', callback as (p: unknown) => void),
    onSchedulerMissed: (callback: (missed: unknown[]) => void) =>
      rpc.on('scheduler:missed', callback as (p: unknown) => void),
    completeConnectorInbox: (params: {
      id: number
      leaseToken: string
      disposition: 'processed' | 'retry' | 'defer'
      error?: string
    }) => rpc.invoke('connector:inboxComplete', params),
    renewConnectorInbox: (params: { id: number; leaseToken: string }) =>
      rpc.invoke('connector:inboxRenew', params),

    // ── Window Controls (no-op in web) ──
    windowMinimize: () => {},
    windowMaximize: () => {},
    windowClose: () => {},

    // ── Widget (fire-and-forget to server) ──
    notifyWidgetStatus: () => rpc.notify('widget:requestUpdate'),
    setWidgetEnabled: (_enabled: boolean) => {},

    // ── Widget Events (no-op in web — no separate widget window) ──
    onWidgetSelectTerminal: (_callback: (terminalId: string) => void) => () => {},

    // ── Workflow Runs ──
    saveWorkflowRun: (execution: unknown) => rpc.invoke('workflowRun:save', execution),
    listWorkflowRuns: (workflowId: string, limit?: number) =>
      rpc.invoke('workflowRun:list', { workflowId, limit }),
    listWorkflowRunsByTask: (taskId: string, limit?: number) =>
      rpc.invoke('workflowRun:listByTask', { taskId, limit }),
    reportWorkflowComplete: (data: unknown) => rpc.invoke('workflow:executionComplete', data),

    // ── Tailscale Network Access ──
    getTailscaleStatus: () => rpc.invoke('tailscale:status'),
    getReachableUrls: () => rpc.invoke('server:reachableUrls'),

    // Connect window (Electron-only). A browser reaches a server by its address,
    // so there is nothing to point somewhere else — it is already there.
    getConnectSettings: async () => null,
    saveConnectSettings: unsupportedInWeb('Pointing at another server'),
    useLocalServer: unsupportedInWeb('Switching to a local server'),

    // Device tokens. A real implementation rather than a stub — unlike SSH keys,
    // these need nothing from Electron, so a phone can manage them too.
    // Pairing. Real here too: the methods need nothing from Electron, and the
    // web client is served from the very machine a phone would be pairing to.
    startPairing: () => rpc.invoke('pairing:start'),
    pendingPairings: () => rpc.invoke('pairing:pending'),
    approvePairing: (requestId: string) => rpc.invoke('pairing:approve', { requestId }),
    denyPairing: (requestId: string) => rpc.invoke('pairing:deny', { requestId }),
    cancelPairing: () => rpc.invoke('pairing:cancel'),
    onPairingCollected: (callback: (event: { requestId: string }) => void) =>
      rpc.on('pairing:collected', callback as (p: unknown) => void),
    onPairingRequested: (
      callback: (request: import('../../shared/src/types').PairingRequest) => void
    ) => rpc.on('pairing:requested', callback as (p: unknown) => void),

    listDeviceTokens: () => rpc.invoke('token:list'),
    createDeviceToken: (name: string) => rpc.invoke('token:create', { name }),
    revokeDeviceToken: (id: string) => rpc.invoke('token:revoke', id),

    // ── App Info (web-specific) ──
    getAppVersion: () => 'web',

    // ── Auto-Update (no-op in web) ──
    // The browser cannot update itself, so the status is permanently
    // `unsupported` and every action is inert. App.tsx seeds from
    // getUpdateStatus() on mount, so this has to answer rather than be absent.
    onUpdateStatus:
      (_callback: (status: import('../../shared/src/types').UpdateStatus) => void) => () => {},
    getUpdateStatus: (): import('../../shared/src/types').UpdateStatus => ({ kind: 'unsupported' }),
    checkForUpdates: () => {},
    downloadUpdate: () => {},
    setUpdateAutoDownload: (_enabled: boolean) => {},
    installUpdate: () => {},
    setUpdateChannel: (_channel: 'stable' | 'beta') => {},

    // ── Git ──
    isGitRepo: (projectPath: string) => rpc.invoke('git:isGitRepo', projectPath),
    getGitBranch: (cwd: string) => rpc.invoke('git:getBranch', cwd),
    checkoutBranch: (cwd: string, branch: string) =>
      rpc.invoke('git:checkoutBranch', { cwd, branch }),
    getWorktreeBranch: (worktreePath: string) => rpc.invoke('git:getWorktreeBranch', worktreePath),
    renameWorktree: (worktreePath: string, newName: string) =>
      rpc.invoke('git:renameWorktree', { worktreePath, newName }),

    // ── Files ──
    listDir: (dirPath: string, remoteHostId?: string) =>
      rpc.invoke('file:listDir', { dirPath, remoteHostId }),
    readFileContent: (filePath: string, maxBytes?: number, remoteHostId?: string) =>
      rpc.invoke('file:readContent', { filePath, maxBytes, remoteHostId }),
    writeFileContent: (filePath: string, content: string, remoteHostId?: string) =>
      rpc.invoke('file:writeContent', { filePath, content, remoteHostId }),

    // ── Shells ──
    listShellExecutables: () => rpc.invoke('shell:listExecutables'),
    listInstalledShells: () => rpc.invoke('shell:listInstalled'),

    // ── Sessions, headless and events ──
    listHeadlessSessions: () => rpc.invoke('headless:list'),
    listSessionEventsBySession: (sessionId: string, limit?: number) =>
      rpc.invoke('sessionEvent:listBySession', { sessionId, limit }),

    // ── Workflow runs ──
    // The two list calls send `{}` rather than nothing, matching what the main
    // process sends — the server reads a property off the params object.
    listAllWorkflowRuns: (workspaceId?: string, limit?: number) =>
      rpc.invoke('workflowRun:listAll', { workspaceId, limit }),
    listRunningWorkflowRuns: () => rpc.invoke('workflowRun:listRunning', {}),
    listRunsWithWaitingGates: () => rpc.invoke('workflowRun:listWaiting', {}),
    claimWorkflowRun: (req: { workflowId: string; params?: string; windowMs?: number }) =>
      rpc.invoke('workflowRun:claim', req),
    releaseWorkflowRun: (req: { workflowId: string; params?: string; runId: string }) =>
      rpc.invoke('workflowRun:release', req),
    runWorkflowManual: (workflowId: string, inputs?: Record<string, unknown>) =>
      rpc.invoke('workflow:runManual', { workflowId, inputs }),

    // ── Connections and connectors ──
    listConnections: (connectorId?: string) => rpc.invoke('connection:list', { connectorId }),
    createConnection: (params: unknown) => rpc.invoke('connection:create', params),
    updateConnection: (id: string, updates: unknown) =>
      rpc.invoke('connection:update', { id, updates }),
    deleteConnection: (id: string) => rpc.invoke('connection:delete', id),
    backfillConnection: (connectionId: string) =>
      rpc.invoke('connection:backfill', { connectionId }),
    listConnectionActions: (connectionId: string) =>
      rpc.invoke('connection:listActions', connectionId),
    executeConnectorAction: (params: {
      connectionId: string
      action: string
      args: Record<string, unknown>
    }) => rpc.invoke('connection:executeAction', params),
    listMcpTools: (connectionId: string) => rpc.invoke('connection:listMcpTools', connectionId),
    refreshMcpTools: (connectionId: string) =>
      rpc.invoke('connection:refreshMcpTools', connectionId),
    getTaskSourceLink: (taskId: string) => rpc.invoke('connection:getSourceLink', taskId),
    upsertTaskFromItem: (params: unknown) => rpc.invoke('connection:upsertFromItem', params),
    listConnectors: () => rpc.invoke('connector:list'),
    getConnector: (id: string) => rpc.invoke('connector:get', id),
    getConnectorStatus: () => rpc.invoke('connector:status'),
    listConnectorCatalog: () => rpc.invoke('connector:catalog'),
    refreshConnectorCatalog: () => rpc.invoke('connector:catalogRefresh'),
    probeSdkConnector: (request: unknown) => rpc.invoke('connector:probeSdk', request),
    detectRepo: (projectPath: string) => rpc.invoke('connector:detectRepo', projectPath),
    seedConnectorWorkflow: (connectionId: string, event: string) =>
      rpc.invoke('connector:seedWorkflow', { connectionId, event }),

    // ── Projects and remote hosts ──
    detectMobileProject: (projectPath: string) =>
      rpc.invoke('project:detectMobile', { projectPath }),
    testSshConnection: (host: unknown) => rpc.invoke('ssh:testConnection', host),

    // ── Opening a link ──
    // The one Electron-only call with a real browser equivalent. `noopener` because
    // the opened page would otherwise get a handle on this one.
    openExternal: async (url: string): Promise<void> => {
      window.open(url, '_blank', 'noopener,noreferrer')
    },

    // ── Window chrome (no-op in web) ──
    // The browser owns the window, so there is nothing to report or subscribe to.
    isWindowMaximized: async (): Promise<boolean> => false,
    onWindowMaximizedChange: (_callback: (maximized: boolean) => void) => () => {},

    // ── Credential vault (unavailable in web) ──
    // Backed by Electron's safeStorage, which is an OS keychain binding with no
    // browser equivalent. `isSafeStorageAvailable` answering false is the honest
    // reply and is what the SSH settings UI already gates on, so the panel explains
    // itself rather than offering controls that would fail. The list is genuinely
    // empty here; the writes reject, because silently discarding a private key the
    // person believed they had saved is the worse failure.
    isSafeStorageAvailable: async (): Promise<boolean> => false,
    listSSHKeys: async () => [],
    storeSSHKey: unsupportedInWeb('Storing an SSH key'),
    importSSHKeyFile: unsupportedInWeb('Importing an SSH key'),
    deleteSSHKey: unsupportedInWeb('Deleting an SSH key'),
    encryptString: unsupportedInWeb('Encrypting a credential'),

    // ── Browser and device panes (unavailable in web) ──
    // Both drive something on the host machine — an embedded Electron view, and a
    // simulator over its local control socket. Neither is reachable from a browser
    // on another device, so the queries answer empty and the actions say why. The
    // subscriptions return an unsubscribe that does nothing: App registers them at
    // mount, and a missing one throws during render and takes the whole app down.
    onBrowserOpenPane: (_callback: (p: { sessionId: string; url?: string }) => void) => () => {},
    onBrowserTabCommand: (_callback: (p: unknown) => void) => () => {},
    onDeviceOpenPane: (_callback: (p: unknown) => void) => () => {},
    attachBrowser: (_sessionId: string, _webContentsId: number, _fileRoot?: string): void => {},
    detachBrowser: (_sessionId: string): void => {},
    syncBrowserTabs: (_sessionId: string, _tabs: unknown[]): void => {},
    // A design artifact is a `<webview>` guest read over CDP, and a browser on
    // another device has neither. Answering "not an artifact" keeps the pane on
    // its address bar rather than leaving a control strip that drives nothing.
    watchBrowserFile: (_sessionId: string, _path: string | null): void => {},
    onBrowserFileChanged: (_callback: (p: unknown) => void) => () => {},
    readBrowserManifest: async () => ({ manifest: null }),
    setBrowserTweak: async () => ({ ok: true as const }),
    cancelBrowserPick: (_sessionId: string): void => {},
    startBrowserPick: async () => null,
    annotateBrowser: unsupportedInWeb('Annotating the browser pane'),
    deviceList: async () => [],
    deviceClaim: unsupportedInWeb('Claiming a device'),
    deviceRelease: async () => ({ released: false }),
    deviceScreenshot: unsupportedInWeb('Taking a device screenshot'),
    deviceInteract: unsupportedInWeb('Interacting with a device'),
    pickDeviceElement: unsupportedInWeb('Picking a device element'),
    annotateDevice: unsupportedInWeb('Annotating the device pane')
  }

  return api
}

export type WebApiShim = ReturnType<typeof createApiShim>

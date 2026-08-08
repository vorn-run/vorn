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

class RpcClient {
  private ws!: WebSocket
  private nextId = 1
  private pending = new Map<number, PendingRequest>()
  private listeners = new Map<string, Set<(params: unknown) => void>>()
  private url: string
  private _ready: Promise<void>
  private _resolveReady!: () => void
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null

  constructor(url: string) {
    this.url = url
    this._ready = new Promise((resolve) => {
      this._resolveReady = resolve
    })
    this.connect()
  }

  private connect(): void {
    this.ws = new WebSocket(this.url)

    this.ws.onopen = () => {
      this._resolveReady()
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

    this.ws.onclose = () => {
      // Reject all pending requests
      for (const [, p] of this.pending) {
        p.reject(new Error('WebSocket disconnected'))
      }
      this.pending.clear()

      // Auto-reconnect after 2s
      this.reconnectTimer = setTimeout(() => {
        this._ready = new Promise((resolve) => {
          this._resolveReady = resolve
        })
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

    // ── Terminal Management ──
    createTerminal: (payload: unknown) => rpc.invoke('terminal:create', payload),
    writeTerminal: (id: string, data: string) => rpc.notify('terminal:write', { id, data }),
    resizeTerminal: (payload: unknown) => rpc.notify('terminal:resize', payload),
    killTerminal: (id: string) => rpc.invoke('terminal:kill', id),
    createShellTerminal: (cwd?: string) => rpc.invoke('shell:create', cwd),

    // ── Terminal Events ──
    onTerminalData: (callback: (event: { id: string; data: string }) => void) =>
      rpc.on('terminal:data', callback as (p: unknown) => void),
    onTerminalExit: (callback: (event: { id: string; exitCode: number }) => void) =>
      rpc.on('terminal:exit', callback as (p: unknown) => void),
    onSessionCreated: (callback: (session: unknown) => void) =>
      rpc.on('session:created', callback as (p: unknown) => void),

    // ── Configuration ──
    loadConfig: () => rpc.invoke('config:load'),
    saveConfig: (config: unknown) => rpc.invoke('config:save', config),
    onConfigChanged: (callback: (config: unknown) => void) =>
      rpc.on('config:changed', callback as (p: unknown) => void),

    // ── Menu Events (Electron-only, no-op in web) ──
    onMenuNewAgent: (_callback: () => void) => () => {},

    // ── Sessions ──
    listActiveSessions: () => rpc.invoke('terminal:listActive'),
    getPreviousSessions: () => rpc.invoke('sessions:getPrevious'),
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

    // ── App Info (web-specific) ──
    getAppVersion: () => 'web',

    // ── Auto-Update (no-op in web) ──
    onUpdateDownloaded: (_callback: (info: { version: string }) => void) => () => {},
    installUpdate: () => {},
    setUpdateChannel: (_channel: 'stable' | 'beta') => {}
  }

  return api
}

export type WebApiShim = ReturnType<typeof createApiShim>

import { contextBridge, ipcRenderer } from 'electron'
import {
  LOCAL_SERVER_RUNNING_CHANNEL,
  SERVER_REPLACED_CHANNEL,
  STOP_SESSIONS_AND_SERVER_CHANNEL,
  type LocalServerNotice
} from '../shared/adoption-channels'
import { captureViewerSettings, withViewerSettings } from '@vornrun/shared/viewer-settings-store'
import {
  CreateTerminalPayload,
  TerminalSession,
  RestoredSession,
  ResizePayload,
  AppConfig,
  RecentSession,
  IPC,
  GitDiffStat,
  GitDiffResult,
  GitCommitPayload,
  GitCommitResult,
  ScheduleLogEntry,
  HeadlessSession,
  WorkflowExecution,
  ScriptConfig,
  AiAgentType,
  MobileProject,
  SSHKeyMeta,
  RemoteHost,
  TailscaleStatus,
  ReachableUrls,
  DeviceToken,
  PairingRequest,
  FileEntry,
  SourceConnection,
  TaskSourceLink,
  ConnectorManifest,
  ConnectorActionDef,
  InstalledShell,
  WorktreeInventory,
  WorktreeActionResult,
  BranchDeleteResult,
  ArtifactManifest,
  BrowserSelection,
  BrowserStroke,
  BrowserTabInfo,
  BrowserAnnotation,
  DeviceInfo,
  DeviceSelection,
  DeviceAnnotation,
  DeviceTarget,
  DevicePoint,
  UpdateStatus
} from '../shared/types'

const api = {
  createTerminal: (payload: CreateTerminalPayload) =>
    ipcRenderer.invoke(IPC.TERMINAL_CREATE, payload),

  writeTerminal: (id: string, data: string) => ipcRenderer.send(IPC.TERMINAL_WRITE, { id, data }),

  resizeTerminal: (payload: ResizePayload) => ipcRenderer.send(IPC.TERMINAL_RESIZE, payload),

  killTerminal: (id: string) => ipcRenderer.invoke(IPC.TERMINAL_KILL, id),

  /** Everything a pane needs to show a terminal it did not create. */
  attachTerminal: (id: string): Promise<{ data: string; seq: number; live: boolean }> =>
    ipcRenderer.invoke(IPC.TERMINAL_ATTACH, id),

  /**
   * What the server has, which is not the same as what the database remembers.
   *
   * The web client has asked this since it was written; the desktop never could,
   * which is why its start-up read the saved list and relaunched from it. It is
   * the same question and it deserves the same answer.
   */
  listActiveSessions: (): Promise<TerminalSession[]> =>
    ipcRenderer.invoke(IPC.TERMINAL_LIST_ACTIVE),

  createShellTerminal: (cwd?: string): Promise<TerminalSession> =>
    ipcRenderer.invoke(IPC.SHELL_CREATE, cwd),

  onTerminalData: (callback: (event: { id: string; data: string; seq: number }) => void) => {
    const listener = (
      _: Electron.IpcRendererEvent,
      event: { id: string; data: string; seq: number }
    ): void => callback(event)
    ipcRenderer.on(IPC.TERMINAL_DATA, listener)
    return () => {
      ipcRenderer.removeListener(IPC.TERMINAL_DATA, listener)
    }
  },

  onTerminalExit: (callback: (event: { id: string; exitCode: number }) => void) => {
    const listener = (
      _: Electron.IpcRendererEvent,
      event: { id: string; exitCode: number }
    ): void => callback(event)
    ipcRenderer.on(IPC.TERMINAL_EXIT, listener)
    return () => {
      ipcRenderer.removeListener(IPC.TERMINAL_EXIT, listener)
    }
  },

  onSessionCreated: (callback: (session: TerminalSession) => void) => {
    const listener = (_: Electron.IpcRendererEvent, session: TerminalSession): void =>
      callback(session)
    ipcRenderer.on(IPC.SESSION_CREATED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.SESSION_CREATED, listener)
    }
  },

  // Settings that belong to this device are laid over the server's on the way in
  // and remembered on the way out, so two clients on one server stop overwriting
  // each other's view mode, font size and workspace. See config-scope.ts for which
  // keys those are and why. Done here rather than in the renderer so both
  // transports behave identically and no call site has to know.
  loadConfig: async (): Promise<AppConfig> =>
    withViewerSettings(await ipcRenderer.invoke(IPC.CONFIG_LOAD)),

  saveConfig: (config: AppConfig) => {
    captureViewerSettings(config)
    return ipcRenderer.invoke(IPC.CONFIG_SAVE, config)
  },

  onConfigChanged: (callback: (config: AppConfig) => void) => {
    const listener = (_: Electron.IpcRendererEvent, config: AppConfig): void =>
      callback(withViewerSettings(config))
    ipcRenderer.on(IPC.CONFIG_CHANGED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.CONFIG_CHANGED, listener)
    }
  },

  /**
   * The server behind this app has been replaced by a different process.
   *
   * Sent after a crash-relaunch. Everything the old one was holding is gone, so
   * a pane showing a terminal is showing a photograph and does not know it.
   */
  onServerReplaced: (callback: () => void) => {
    const listener = (): void => callback()
    ipcRenderer.on(SERVER_REPLACED_CHANNEL, listener)
    return () => {
      ipcRenderer.removeListener(SERVER_REPLACED_CHANNEL, listener)
    }
  },

  onLocalServerStillRunning: (callback: (notice: LocalServerNotice) => void) => {
    const listener = (_: Electron.IpcRendererEvent, notice: LocalServerNotice): void =>
      callback(notice)
    ipcRenderer.on(LOCAL_SERVER_RUNNING_CHANNEL, listener)
    return () => {
      ipcRenderer.removeListener(LOCAL_SERVER_RUNNING_CHANNEL, listener)
    }
  },

  onMenuNewAgent: (callback: () => void) => {
    const listener = (): void => callback()
    ipcRenderer.on('menu:new-agent', listener)
    return () => {
      ipcRenderer.removeListener('menu:new-agent', listener)
    }
  },

  /** Sessions from the last run that no pane has taken yet. */
  getRestoredSessions: (): Promise<RestoredSession[]> => ipcRenderer.invoke(IPC.SESSIONS_RESTORED),

  /** Claim one and start it. `gone` means another pane took it first. */
  resumeSession: (params: {
    id: string
    resumeSessionId?: string
  }): Promise<
    { ok: true; session: TerminalSession } | { ok: false; reason: string; message?: string }
  > => ipcRenderer.invoke(IPC.SESSIONS_RESUME, params),

  clearPreviousSessions: () => ipcRenderer.invoke(IPC.SESSIONS_CLEAR),

  getRecentSessions: (projectPath?: string): Promise<RecentSession[]> =>
    ipcRenderer.invoke(IPC.SESSIONS_GET_RECENT, projectPath),

  renameSession: (id: string, displayName: string) =>
    ipcRenderer.invoke(IPC.TERMINAL_RENAME, { id, displayName }),

  reorderSessions: (ids: string[]) => ipcRenderer.invoke(IPC.TERMINAL_REORDER, ids),

  openDirectoryDialog: (): Promise<string | null> => ipcRenderer.invoke(IPC.DIALOG_OPEN_DIRECTORY),

  openFileDialog: (): Promise<string | null> => ipcRenderer.invoke(IPC.DIALOG_OPEN_FILE),

  detectIDEs: (): Promise<{ id: string; name: string; command: string }[]> =>
    ipcRenderer.invoke(IPC.IDE_DETECT),

  /**
   * Advisory: whether this project looks like a mobile app, so the device
   * control can be offered where it helps. Never a gate on the device tools —
   * an agent asked to drive a simulator for some other directory must still be
   * able to.
   */
  detectMobileProject: (projectPath: string): Promise<MobileProject> =>
    ipcRenderer.invoke(IPC.PROJECT_DETECT_MOBILE, { projectPath }),

  detectInstalledAgents: (): Promise<Record<AiAgentType, boolean>> =>
    ipcRenderer.invoke(IPC.AGENT_DETECT_INSTALLED),

  openInIDE: (ideId: string, projectPath: string): Promise<void> =>
    ipcRenderer.invoke(IPC.IDE_OPEN, { ideId, projectPath }),

  isGitRepo: (projectPath: string): Promise<boolean> =>
    ipcRenderer.invoke(IPC.GIT_IS_REPO, projectPath),

  listBranches: (
    projectPath: string
  ): Promise<{ local: string[]; current: string | null; isGitRepo: boolean }> =>
    ipcRenderer.invoke(IPC.GIT_LIST_BRANCHES, projectPath),

  listRemoteBranches: (projectPath: string): Promise<string[]> =>
    ipcRenderer.invoke(IPC.GIT_LIST_REMOTE_BRANCHES, projectPath),

  createWorktree: (
    projectPath: string,
    branch: string,
    worktreeName?: string
  ): Promise<{ worktreePath: string; branch: string; name: string }> =>
    ipcRenderer.invoke(IPC.GIT_CREATE_WORKTREE, { projectPath, branch, worktreeName }),

  removeWorktree: (
    projectPath: string,
    worktreePath: string,
    force?: boolean,
    deleteBranch?: boolean
  ): Promise<boolean> =>
    ipcRenderer.invoke(IPC.GIT_REMOVE_WORKTREE, { projectPath, worktreePath, force, deleteBranch }),

  renameWorktreeBranch: (worktreePath: string, newBranch: string): Promise<boolean> =>
    ipcRenderer.invoke(IPC.GIT_RENAME_WORKTREE_BRANCH, { worktreePath, newBranch }),

  renameWorktree: (
    worktreePath: string,
    newName: string
  ): Promise<{ newPath: string; name: string } | null> =>
    ipcRenderer.invoke(IPC.GIT_RENAME_WORKTREE, { worktreePath, newName }),

  isWorktreeDirty: (worktreePath: string): Promise<boolean> =>
    ipcRenderer.invoke(IPC.GIT_WORKTREE_DIRTY, worktreePath),

  listWorktrees: (
    projectPath: string
  ): Promise<{ path: string; branch: string; isMain: boolean; name: string }[]> =>
    ipcRenderer.invoke(IPC.GIT_LIST_WORKTREES, projectPath),

  checkoutBranch: (cwd: string, branch: string): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.GIT_CHECKOUT_BRANCH, { cwd, branch }),

  getWorktreeBranch: (worktreePath: string): Promise<string | null> =>
    ipcRenderer.invoke(IPC.GIT_GET_WORKTREE_BRANCH, worktreePath),

  getWorktreeActiveSessions: (
    worktreePath: string
  ): Promise<{ count: number; sessionIds: string[] }> =>
    ipcRenderer.invoke(IPC.WORKTREE_ACTIVE_SESSIONS, worktreePath),

  getWorktreeInventory: (params?: {
    projectPaths?: string[]
    refresh?: boolean
  }): Promise<WorktreeInventory> => ipcRenderer.invoke(IPC.WORKTREE_INVENTORY, params),

  reclaimWorktreeArtifacts: (paths: string[]): Promise<WorktreeActionResult> =>
    ipcRenderer.invoke(IPC.WORKTREE_RECLAIM_ARTIFACTS, { paths }),

  removeWorktrees: (
    items: {
      projectPath: string
      worktreePath: string
      force?: boolean
      deleteBranch?: boolean
    }[]
  ): Promise<WorktreeActionResult> => ipcRenderer.invoke(IPC.WORKTREE_REMOVE_MANY, { items }),

  pruneOrphanWorktrees: (paths: string[]): Promise<WorktreeActionResult> =>
    ipcRenderer.invoke(IPC.WORKTREE_PRUNE_ORPHANS, { paths }),

  deleteBranches: (
    projectPath: string,
    branches: string[],
    force?: boolean
  ): Promise<BranchDeleteResult> =>
    ipcRenderer.invoke(IPC.GIT_DELETE_BRANCHES, { projectPath, branches, force }),

  getGitBranch: (cwd: string): Promise<string | null> =>
    ipcRenderer.invoke(IPC.GIT_GET_BRANCH, cwd),

  getGitDiffStat: (cwd: string): Promise<GitDiffStat | null> =>
    ipcRenderer.invoke(IPC.GIT_DIFF_STAT, cwd),

  getGitDiffFull: (cwd: string): Promise<GitDiffResult | null> =>
    ipcRenderer.invoke(IPC.GIT_DIFF_FULL, cwd),

  gitCommit: (payload: GitCommitPayload): Promise<GitCommitResult> =>
    ipcRenderer.invoke(IPC.GIT_COMMIT, payload),

  gitPush: (cwd: string): Promise<GitCommitResult> => ipcRenderer.invoke(IPC.GIT_PUSH, cwd),

  // File explorer
  listDir: (dirPath: string, remoteHostId?: string): Promise<FileEntry[]> =>
    ipcRenderer.invoke(IPC.FILE_LIST_DIR, { dirPath, remoteHostId }),
  listShellExecutables: (): Promise<string[]> => ipcRenderer.invoke(IPC.SHELL_LIST_EXECUTABLES),
  listInstalledShells: (): Promise<InstalledShell[]> =>
    ipcRenderer.invoke(IPC.SHELL_LIST_INSTALLED),
  readFileContent: (
    filePath: string,
    maxBytes?: number,
    remoteHostId?: string
  ): Promise<string | null> =>
    ipcRenderer.invoke(IPC.FILE_READ_CONTENT, { filePath, maxBytes, remoteHostId }),
  writeFileContent: (
    filePath: string,
    content: string,
    remoteHostId?: string
  ): Promise<{ success: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.FILE_WRITE_CONTENT, { filePath, content, remoteHostId }),

  // Task images
  openImageDialog: (): Promise<string[] | null> => ipcRenderer.invoke(IPC.DIALOG_OPEN_IMAGE),

  saveTaskImage: (taskId: string, sourcePath: string): Promise<string> =>
    ipcRenderer.invoke(IPC.TASK_IMAGE_SAVE, { taskId, sourcePath }),

  deleteTaskImage: (taskId: string, filename: string): Promise<void> =>
    ipcRenderer.invoke(IPC.TASK_IMAGE_DELETE, { taskId, filename }),

  getTaskImagePath: (taskId: string, filename: string): Promise<string> =>
    ipcRenderer.invoke(IPC.TASK_IMAGE_GET_PATH, { taskId, filename }),

  cleanupTaskImages: (taskId: string): Promise<void> =>
    ipcRenderer.invoke(IPC.TASK_IMAGE_CLEANUP, taskId),

  // Headless sessions
  createHeadlessSession: (payload: CreateTerminalPayload): Promise<HeadlessSession> =>
    ipcRenderer.invoke(IPC.HEADLESS_CREATE, payload),

  killHeadlessSession: (id: string): Promise<void> => ipcRenderer.invoke(IPC.HEADLESS_KILL, id),

  listHeadlessSessions: (): Promise<HeadlessSession[]> => ipcRenderer.invoke(IPC.HEADLESS_LIST),

  onHeadlessData: (callback: (event: { id: string; data: string }) => void) => {
    const listener = (_: Electron.IpcRendererEvent, event: { id: string; data: string }): void =>
      callback(event)
    ipcRenderer.on(IPC.HEADLESS_DATA, listener)
    return () => {
      ipcRenderer.removeListener(IPC.HEADLESS_DATA, listener)
    }
  },

  onHeadlessExit: (callback: (event: { id: string; exitCode: number }) => void) => {
    const listener = (
      _: Electron.IpcRendererEvent,
      event: { id: string; exitCode: number }
    ): void => callback(event)
    ipcRenderer.on(IPC.HEADLESS_EXIT, listener)
    return () => {
      ipcRenderer.removeListener(IPC.HEADLESS_EXIT, listener)
    }
  },

  executeScript: (
    config: ScriptConfig
  ): Promise<{ success: boolean; output: string; error?: string; exitCode?: number }> =>
    ipcRenderer.invoke(IPC.SCRIPT_EXECUTE, config),

  onScriptData: (callback: (event: { runId: string; data: string }) => void) => {
    const listener = (_: Electron.IpcRendererEvent, event: { runId: string; data: string }): void =>
      callback(event)
    ipcRenderer.on(IPC.SCRIPT_DATA, listener)
    return () => {
      ipcRenderer.removeListener(IPC.SCRIPT_DATA, listener)
    }
  },

  onScriptExit: (callback: (event: { runId: string; exitCode: number }) => void) => {
    const listener = (
      _: Electron.IpcRendererEvent,
      event: { runId: string; exitCode: number }
    ): void => callback(event)
    ipcRenderer.on(IPC.SCRIPT_EXIT, listener)
    return () => {
      ipcRenderer.removeListener(IPC.SCRIPT_EXIT, listener)
    }
  },

  /**
   * Main asking for a browser pane to open, on the agent's behalf.
   *
   * Panes live in renderer state, so this is the only way an agent can get one
   * without a person clicking. Fire-and-forget; main waits for the attach
   * report that follows rather than for a reply here.
   */
  onBrowserOpenPane: (callback: (p: { sessionId: string; url?: string }) => void) => {
    const listener = (_: Electron.IpcRendererEvent, p: { sessionId: string; url?: string }): void =>
      callback(p)
    ipcRenderer.on(IPC.BROWSER_OPEN_PANE, listener)
    return () => {
      ipcRenderer.removeListener(IPC.BROWSER_OPEN_PANE, listener)
    }
  },

  /**
   * Main asking for a device pane to open, on the agent's behalf.
   *
   * The claim already exists by the time this fires — the pane is a viewer, so
   * it can never be what grants a session its device.
   */
  onDeviceOpenPane: (callback: (p: { sessionId: string; udid: string; name: string }) => void) => {
    const listener = (
      _: Electron.IpcRendererEvent,
      p: { sessionId: string; udid: string; name: string }
    ): void => callback(p)
    ipcRenderer.on(IPC.DEVICE_OPEN_PANE, listener)
    return () => {
      ipcRenderer.removeListener(IPC.DEVICE_OPEN_PANE, listener)
    }
  },

  /** Main asking to add, close, or switch a tab in a pane. */
  onBrowserTabCommand: (
    callback: (p: {
      sessionId: string
      action: 'add' | 'close' | 'select'
      url?: string
      index?: number
    }) => void
  ) => {
    const listener = (_: Electron.IpcRendererEvent, p: Parameters<typeof callback>[0]): void =>
      callback(p)
    ipcRenderer.on(IPC.BROWSER_TAB_COMMAND, listener)
    return () => {
      ipcRenderer.removeListener(IPC.BROWSER_TAB_COMMAND, listener)
    }
  },

  onSessionUpdated: (callback: (session: TerminalSession) => void) => {
    const listener = (_: Electron.IpcRendererEvent, session: TerminalSession): void =>
      callback(session)
    ipcRenderer.on(IPC.SESSION_UPDATED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.SESSION_UPDATED, listener)
    }
  },

  onWorktreeCleanup: (
    callback: (session: { id: string; projectPath: string; worktreePath: string }) => void
  ) => {
    const listener = (
      _: Electron.IpcRendererEvent,
      session: { id: string; projectPath: string; worktreePath: string }
    ): void => callback(session)
    ipcRenderer.on(IPC.WORKTREE_CONFIRM_CLEANUP, listener)
    return () => {
      ipcRenderer.removeListener(IPC.WORKTREE_CONFIRM_CLEANUP, listener)
    }
  },

  // Scheduler APIs
  getScheduleLog: (workflowId?: string): Promise<ScheduleLogEntry[]> =>
    ipcRenderer.invoke(IPC.SCHEDULER_GET_LOG, workflowId),

  getScheduleNextRun: (workflowId: string): Promise<string | null> =>
    ipcRenderer.invoke(IPC.SCHEDULER_GET_NEXT_RUN, workflowId),

  onSchedulerExecute: (
    callback: (event: {
      workflowId: string
      connectorItem?: import('../../packages/shared/src/types').ConnectorItemContext
      inputs?: Record<string, unknown>
      connectorInboxId?: number
      connectorInboxLeaseToken?: string
      existingExecution?: import('../../packages/shared/src/types').WorkflowExecution
    }) => void
  ) => {
    const listener = (
      _: Electron.IpcRendererEvent,
      event: {
        workflowId: string
        connectorItem?: import('../../packages/shared/src/types').ConnectorItemContext
        inputs?: Record<string, unknown>
        connectorInboxId?: number
        connectorInboxLeaseToken?: string
        existingExecution?: import('../../packages/shared/src/types').WorkflowExecution
      }
    ): void => callback(event)
    ipcRenderer.on(IPC.SCHEDULER_EXECUTE, listener)
    return () => {
      ipcRenderer.removeListener(IPC.SCHEDULER_EXECUTE, listener)
    }
  },

  onSchedulerStopRun: (callback: (event: { runId: string }) => void) => {
    const listener = (_: Electron.IpcRendererEvent, event: { runId: string }): void =>
      callback(event)
    ipcRenderer.on(IPC.SCHEDULER_STOP_RUN, listener)
    return () => {
      ipcRenderer.removeListener(IPC.SCHEDULER_STOP_RUN, listener)
    }
  },
  onWorkflowGateResolved: (
    callback: (event: { runId: string; nodeId: string; decision: 'approve' | 'reject' }) => void
  ) => {
    const listener = (
      _e: unknown,
      event: { runId: string; nodeId: string; decision: 'approve' | 'reject' }
    ) => callback(event)
    ipcRenderer.on(IPC.WORKFLOW_GATE_RESOLVED, listener)
    return () => ipcRenderer.removeListener(IPC.WORKFLOW_GATE_RESOLVED, listener)
  },

  onSchedulerMissed: (
    callback: (missed: { workflow: { id: string; name: string }; scheduledFor: string }[]) => void
  ) => {
    const listener = (
      _: Electron.IpcRendererEvent,
      missed: { workflow: { id: string; name: string }; scheduledFor: string }[]
    ): void => callback(missed)
    ipcRenderer.on(IPC.SCHEDULER_MISSED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.SCHEDULER_MISSED, listener)
    }
  },

  // Window controls (Windows/Linux custom titlebar)
  windowMinimize: () => ipcRenderer.send(IPC.WINDOW_MINIMIZE),
  windowMaximize: () => ipcRenderer.send(IPC.WINDOW_MAXIMIZE),
  windowClose: () => ipcRenderer.send(IPC.WINDOW_CLOSE),
  isWindowMaximized: (): Promise<boolean> => ipcRenderer.invoke(IPC.WINDOW_IS_MAXIMIZED),
  onWindowMaximizedChange: (callback: (maximized: boolean) => void) => {
    const listener = (_: Electron.IpcRendererEvent, maximized: boolean): void => callback(maximized)
    ipcRenderer.on(IPC.WINDOW_MAXIMIZED_CHANGED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.WINDOW_MAXIMIZED_CHANGED, listener)
    }
  },

  // Widget
  notifyWidgetStatus: () => ipcRenderer.send(IPC.WIDGET_RENDERER_STATUS),
  setWidgetEnabled: (enabled: boolean) => ipcRenderer.send(IPC.WIDGET_SET_ENABLED, enabled),

  onWidgetSelectTerminal: (callback: (terminalId: string) => void) => {
    const listener = (_: Electron.IpcRendererEvent, terminalId: string): void =>
      callback(terminalId)
    ipcRenderer.on('widget:select-terminal', listener)
    return () => {
      ipcRenderer.removeListener('widget:select-terminal', listener)
    }
  },

  // Session events (lifecycle log: created / exited / renamed)
  listSessionEventsBySession: (
    sessionId: string,
    limit?: number
  ): Promise<import('../shared/types').SessionEvent[]> =>
    ipcRenderer.invoke(IPC.SESSION_EVENT_LIST_BY_SESSION, sessionId, limit),

  // Workflow runs
  saveWorkflowRun: (execution: WorkflowExecution): Promise<void> =>
    ipcRenderer.invoke(IPC.WORKFLOW_RUN_SAVE, execution),

  listWorkflowRuns: (workflowId: string, limit?: number): Promise<WorkflowExecution[]> =>
    ipcRenderer.invoke(IPC.WORKFLOW_RUN_LIST, workflowId, limit),

  listWorkflowRunsByTask: (
    taskId: string,
    limit?: number
  ): Promise<(WorkflowExecution & { workflowName?: string })[]> =>
    ipcRenderer.invoke(IPC.WORKFLOW_RUN_LIST_BY_TASK, taskId, limit),

  listRunsWithWaitingGates: (): Promise<WorkflowExecution[]> =>
    ipcRenderer.invoke(IPC.WORKFLOW_RUN_LIST_WAITING),

  listRunningWorkflowRuns: (): Promise<WorkflowExecution[]> =>
    ipcRenderer.invoke(IPC.WORKFLOW_RUN_LIST_RUNNING),

  listAllWorkflowRuns: (
    workspaceId?: string,
    limit?: number
  ): Promise<(WorkflowExecution & { workflowName?: string })[]> =>
    ipcRenderer.invoke(IPC.WORKFLOW_RUN_LIST_ALL, workspaceId, limit),

  reportWorkflowComplete: (data: {
    workflowId: string
    workflowName: string
    completedAt: string
    status: 'success' | 'error' | 'cancelled'
    sessionsLaunched: number
    source?: 'scheduler' | 'manual'
  }): Promise<void> => ipcRenderer.invoke(IPC.WORKFLOW_EXECUTION_COMPLETE, data),

  /**
   * Ask the core for the right to run this trigger. Every instance shares one
   * core, so this is where a tick broadcast to several windows collapses into
   * a single run. Returns the run id to use when granted.
   */
  claimWorkflowRun: (req: {
    workflowId: string
    params?: string
    windowMs?: number
  }): Promise<{ granted: boolean; runId: string }> =>
    ipcRenderer.invoke(IPC.WORKFLOW_RUN_CLAIM, req),

  releaseWorkflowRun: (req: {
    workflowId: string
    params?: string
    runId: string
  }): Promise<void> => ipcRenderer.invoke(IPC.WORKFLOW_RUN_RELEASE, req),

  // Credential vault
  storeSSHKey: (params: {
    label: string
    privateKey: string
    publicKey?: string
    certificate?: string
  }): Promise<{ id: string }> => ipcRenderer.invoke(IPC.CREDENTIAL_STORE_KEY, params),

  importSSHKeyFile: (params: { filePath: string; label?: string }): Promise<{ id: string }> =>
    ipcRenderer.invoke(IPC.CREDENTIAL_IMPORT_KEY_FILE, params),

  deleteSSHKey: (id: string): Promise<void> => ipcRenderer.invoke(IPC.CREDENTIAL_DELETE_KEY, id),

  listSSHKeys: (): Promise<SSHKeyMeta[]> => ipcRenderer.invoke(IPC.CREDENTIAL_LIST_KEYS),

  encryptString: (plaintext: string): Promise<string> =>
    ipcRenderer.invoke(IPC.CREDENTIAL_ENCRYPT, plaintext),

  isSafeStorageAvailable: (): Promise<boolean> =>
    ipcRenderer.invoke(IPC.CREDENTIAL_SAFE_STORAGE_AVAILABLE),

  // Tailscale
  getTailscaleStatus: (): Promise<TailscaleStatus> => ipcRenderer.invoke(IPC.TAILSCALE_STATUS),
  getReachableUrls: (): Promise<ReachableUrls> => ipcRenderer.invoke(IPC.SERVER_REACHABLE_URLS),

  // Connect window. Handled in the main process without a bridge — they are the
  // only methods that work when there is no server to talk to.
  getConnectSettings: (): Promise<{ mode: string; url: string; hasToken: boolean }> =>
    ipcRenderer.invoke('connect:get'),
  saveConnectSettings: (params: {
    url: string
    token: string
  }): Promise<{ ok: boolean; error?: string }> => ipcRenderer.invoke('connect:save', params),
  useLocalServer: (): Promise<{ ok: boolean }> => ipcRenderer.invoke('connect:useLocal'),

  /** Ends the local server on purpose. Offered by the connect window when it
   *  refused to adopt one, and by the app when it is pointed at a host while a
   *  local server is still running. */
  stopLocalServer: (): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke('connect:stopLocal'),

  /** The File menu's "Stop Sessions and Server", offered from the palette too. */
  stopSessionsAndServer: (): Promise<void> => ipcRenderer.invoke(STOP_SESSIONS_AND_SERVER_CHANNEL),

  // Pairing a phone by showing it a code
  startPairing: (): Promise<{ code: string; expiresAt: number }> =>
    ipcRenderer.invoke(IPC.PAIRING_START),
  pendingPairings: (): Promise<PairingRequest[]> => ipcRenderer.invoke(IPC.PAIRING_PENDING),
  approvePairing: (requestId: string): Promise<{ ok: boolean }> =>
    ipcRenderer.invoke(IPC.PAIRING_APPROVE, { requestId }),
  denyPairing: (requestId: string): Promise<{ ok: boolean }> =>
    ipcRenderer.invoke(IPC.PAIRING_DENY, { requestId }),
  cancelPairing: (): Promise<{ ok: boolean }> => ipcRenderer.invoke(IPC.PAIRING_CANCEL),
  onPairingCollected: (callback: (event: { requestId: string }) => void) => {
    const listener = (_: Electron.IpcRendererEvent, event: { requestId: string }): void =>
      callback(event)
    ipcRenderer.on(IPC.PAIRING_COLLECTED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.PAIRING_COLLECTED, listener)
    }
  },
  onPairingRequested: (callback: (request: PairingRequest) => void) => {
    const listener = (_: Electron.IpcRendererEvent, request: PairingRequest): void =>
      callback(request)
    ipcRenderer.on(IPC.PAIRING_REQUESTED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.PAIRING_REQUESTED, listener)
    }
  },

  // Device tokens
  listDeviceTokens: (): Promise<DeviceToken[]> => ipcRenderer.invoke(IPC.TOKEN_LIST),
  createDeviceToken: (name: string): Promise<{ token: DeviceToken; plaintext: string }> =>
    ipcRenderer.invoke(IPC.TOKEN_CREATE, { name }),
  revokeDeviceToken: (id: string): Promise<{ revoked: boolean }> =>
    ipcRenderer.invoke(IPC.TOKEN_REVOKE, id),

  // SSH
  testSshConnection: (
    host: RemoteHost
  ): Promise<{ success: boolean; message: string; durationMs: number }> =>
    ipcRenderer.invoke(IPC.SSH_TEST_CONNECTION, host),

  // Browser pane — tells main which guest belongs to which session, so the
  // agent's browser tools can reach it. Fire-and-forget in both directions.
  attachBrowser: (sessionId: string, webContentsId: number, fileRoot?: string): void =>
    ipcRenderer.send(IPC.BROWSER_ATTACH, { sessionId, webContentsId, fileRoot }),
  detachBrowser: (sessionId: string): void => ipcRenderer.send(IPC.BROWSER_DETACH, sessionId),
  /** Report this session's tab strip, so main can answer a listing from what
   *  the renderer actually holds rather than a copy of its own. */
  syncBrowserTabs: (sessionId: string, tabs: BrowserTabInfo[]): void =>
    ipcRenderer.send(IPC.BROWSER_TABS_CHANGED, { sessionId, tabs }),
  /** Name the design this pane is showing so main can watch it; null stops. */
  watchBrowserFile: (sessionId: string, path: string | null): void =>
    ipcRenderer.send(IPC.BROWSER_WATCH_FILE, { sessionId, path }),
  /** A watched design changed on disk. */
  onBrowserFileChanged: (callback: (p: { sessionId: string; path: string }) => void) => {
    const listener = (_: Electron.IpcRendererEvent, p: { sessionId: string; path: string }): void =>
      callback(p)
    ipcRenderer.on(IPC.BROWSER_FILE_CHANGED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.BROWSER_FILE_CHANGED, listener)
    }
  },
  /** What the loaded page declares itself to be, plus its live tweak values.
   *  `manifest` is null for an ordinary web page, which is most of them. */
  readBrowserManifest: (
    sessionId: string
  ): Promise<{ manifest: ArtifactManifest | null; values?: Record<string, unknown> }> =>
    ipcRenderer.invoke(IPC.BROWSER_READ_MANIFEST, sessionId),
  /** Write one declared tweak value into the page. */
  setBrowserTweak: (sessionId: string, key: string, value: unknown): Promise<{ ok: true }> =>
    ipcRenderer.invoke(IPC.BROWSER_SET_TWEAK, { sessionId, key, value }),
  /** Arm the element picker. Resolves with the pick, or null if cancelled. */
  startBrowserPick: (sessionId: string): Promise<BrowserSelection | null> =>
    ipcRenderer.invoke(IPC.BROWSER_PICK_START, sessionId),
  cancelBrowserPick: (sessionId: string): void =>
    ipcRenderer.send(IPC.BROWSER_PICK_CANCEL, sessionId),
  /** Resolve freehand ink (in *page* coordinates) to the elements under it. */
  annotateBrowser: (params: {
    sessionId: string
    strokes: BrowserStroke[]
  }): Promise<BrowserAnnotation> => ipcRenderer.invoke(IPC.BROWSER_ANNOTATE, params),

  // Device pane. Unlike the browser's, none of this drives a guest in the
  // renderer: a simulator has no in-process view, so the pane asks main for a
  // frame and hands taps back to main. Both go through the same registry the
  // agent's tools use, so the person and the agent share one generation
  // counter rather than each holding a private idea of the screen.
  deviceScreenshot: (
    sessionId: string,
    maxEdge?: number
  ): Promise<{ data: string; scale: number; screen: { width: number; height: number } }> =>
    ipcRenderer.invoke(IPC.DEVICE_SCREENSHOT, { sessionId, maxEdge }),
  deviceInteract: (params: {
    sessionId: string
    action: 'tap' | 'swipe' | 'type' | 'button' | 'press'
    target?: DeviceTarget
    to?: DevicePoint
    text?: string
    duration?: number
    systemGesture?: boolean
  }): Promise<{ ok: true; generation: number }> => ipcRenderer.invoke(IPC.DEVICE_INTERACT, params),
  deviceList: (): Promise<DeviceInfo[]> => ipcRenderer.invoke(IPC.DEVICE_LIST),
  deviceClaim: (
    sessionId: string,
    udid: string
  ): Promise<{ udid: string; name: string; booted: boolean }> =>
    ipcRenderer.invoke(IPC.DEVICE_CLAIM, { sessionId, udid }),
  deviceRelease: (sessionId: string): Promise<{ released: boolean }> =>
    ipcRenderer.invoke(IPC.DEVICE_RELEASE, { sessionId }),
  /** Resolve a point the person tapped in the pane to the element there. The
   *  point is in **points**, never image pixels — the pane converts first. */
  pickDeviceElement: (sessionId: string, point: DevicePoint): Promise<DeviceSelection> =>
    ipcRenderer.invoke(IPC.DEVICE_PICKED, { sessionId, point }),
  annotateDevice: (params: {
    sessionId: string
    strokes: Array<{ points: DevicePoint[] }>
  }): Promise<DeviceAnnotation> => ipcRenderer.invoke(IPC.DEVICE_ANNOTATE, params),

  // Shell
  openExternal: (url: string): Promise<void> => ipcRenderer.invoke(IPC.OPEN_EXTERNAL, url),

  // App info
  getAppVersion: (): string => ipcRenderer.sendSync('get-app-version'),
  // Auto-update
  onUpdateStatus: (callback: (status: UpdateStatus) => void) => {
    const listener = (_: Electron.IpcRendererEvent, status: UpdateStatus): void => callback(status)
    ipcRenderer.on(IPC.UPDATE_STATUS, listener)
    return () => {
      ipcRenderer.removeListener(IPC.UPDATE_STATUS, listener)
    }
  },
  getUpdateStatus: (): UpdateStatus => ipcRenderer.sendSync(IPC.UPDATE_GET_STATUS),
  checkForUpdates: () => ipcRenderer.send(IPC.UPDATE_CHECK),
  downloadUpdate: () => ipcRenderer.send(IPC.UPDATE_DOWNLOAD),
  setUpdateAutoDownload: (enabled: boolean) =>
    ipcRenderer.send(IPC.UPDATE_SET_AUTO_DOWNLOAD, enabled),
  installUpdate: () => ipcRenderer.send(IPC.UPDATE_INSTALL),
  setUpdateChannel: (channel: 'stable' | 'beta') =>
    ipcRenderer.send(IPC.UPDATE_SET_CHANNEL, channel),

  // Connectors
  listConnectors: (): Promise<
    Array<{
      id: string
      name: string
      icon: string
      capabilities: string[]
      manifest: ConnectorManifest
    }>
  > => ipcRenderer.invoke(IPC.CONNECTOR_LIST),

  getConnector: (
    id: string
  ): Promise<{
    id: string
    name: string
    icon: string
    capabilities: string[]
    manifest: ConnectorManifest
  } | null> => ipcRenderer.invoke(IPC.CONNECTOR_GET, id),

  listConnections: (connectorId?: string): Promise<SourceConnection[]> =>
    ipcRenderer.invoke(IPC.CONNECTION_LIST, { connectorId }),

  createConnection: (
    params: Omit<
      SourceConnection,
      'id' | 'createdAt' | 'lastSyncAt' | 'lastSyncError' | 'syncCursor'
    > & { seedWorkflow?: { name: string; defaultCronFromMinutes: number } }
  ): Promise<SourceConnection> => ipcRenderer.invoke(IPC.CONNECTION_CREATE, params),

  updateConnection: (
    id: string,
    updates: Partial<SourceConnection>
  ): Promise<SourceConnection | null> => ipcRenderer.invoke(IPC.CONNECTION_UPDATE, { id, updates }),

  deleteConnection: (id: string): Promise<void> => ipcRenderer.invoke(IPC.CONNECTION_DELETE, id),

  runWorkflowManual: (workflowId: string, inputs?: Record<string, unknown>): Promise<void> =>
    ipcRenderer.invoke(IPC.WORKFLOW_RUN_MANUAL, { workflowId, inputs }),

  backfillConnection: (
    connectionId: string
  ): Promise<{ imported: number; updated: number; error?: string }> =>
    ipcRenderer.invoke(IPC.CONNECTION_BACKFILL, { connectionId }),

  executeConnectorAction: (params: {
    connectionId: string
    action: string
    args: Record<string, unknown>
  }): Promise<{ success: boolean; output?: Record<string, unknown>; error?: string }> =>
    ipcRenderer.invoke(IPC.CONNECTION_EXECUTE_ACTION, params),

  listConnectionActions: (connectionId: string): Promise<ConnectorActionDef[]> =>
    ipcRenderer.invoke(IPC.CONNECTION_LIST_ACTIONS, connectionId),

  listMcpTools: (
    connectionId: string
  ): Promise<
    Array<{ name: string; description?: string; inputSchema?: Record<string, unknown> }>
  > => ipcRenderer.invoke(IPC.CONNECTION_LIST_MCP_TOOLS, connectionId),

  refreshMcpTools: (
    connectionId: string
  ): Promise<{ ok: boolean; count?: number; error?: string }> =>
    ipcRenderer.invoke(IPC.CONNECTION_REFRESH_MCP_TOOLS, connectionId),

  probeSdkConnector: (
    request: import('../../packages/shared/src/types').SdkProbeRequest
  ): Promise<import('../../packages/shared/src/types').SdkProbeResult> =>
    ipcRenderer.invoke(IPC.CONNECTOR_PROBE_SDK, request),

  listConnectorCatalog: (): Promise<
    import('../../packages/shared/src/types').ConnectorCatalogSnapshot
  > => ipcRenderer.invoke(IPC.CONNECTOR_CATALOG),

  refreshConnectorCatalog: (): Promise<
    import('../../packages/shared/src/types').ConnectorCatalogSnapshot
  > => ipcRenderer.invoke(IPC.CONNECTOR_CATALOG_REFRESH),

  upsertTaskFromItem: (params: {
    connectionId: string
    item: import('../../packages/shared/src/types').ConnectorItemContext
    initialStatus: import('../../packages/shared/src/types').TaskStatus
    project?: string
  }): Promise<{ taskId: string; created: boolean }> =>
    ipcRenderer.invoke(IPC.CONNECTION_UPSERT_FROM_ITEM, params),

  completeConnectorInbox: (params: {
    id: number
    leaseToken: string
    disposition: 'processed' | 'retry' | 'defer'
    error?: string
  }): Promise<void> => ipcRenderer.invoke(IPC.CONNECTOR_INBOX_COMPLETE, params),

  renewConnectorInbox: (params: { id: number; leaseToken: string }): Promise<boolean> =>
    ipcRenderer.invoke(IPC.CONNECTOR_INBOX_RENEW, params),

  getTaskSourceLink: (taskId: string): Promise<TaskSourceLink | null> =>
    ipcRenderer.invoke(IPC.CONNECTION_GET_SOURCE_LINK, taskId),

  detectRepo: (projectPath: string): Promise<{ owner: string; repo: string } | null> =>
    ipcRenderer.invoke(IPC.CONNECTOR_DETECT_REPO, projectPath),

  seedConnectorWorkflow: (
    connectionId: string,
    event: string
  ): Promise<{ workflowId: string; created: boolean }> =>
    ipcRenderer.invoke(IPC.CONNECTOR_SEED_WORKFLOW, { connectionId, event }),

  getConnectorStatus: (): Promise<
    Array<{ connectorId: string; authed: boolean; message?: string }>
  > => ipcRenderer.invoke(IPC.CONNECTOR_STATUS)
}

contextBridge.exposeInMainWorld('api', api)

export type VornAPI = typeof api

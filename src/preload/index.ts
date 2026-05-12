import { contextBridge, ipcRenderer } from 'electron'
import {
  CreateTerminalPayload,
  TerminalSession,
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
  SSHKeyMeta,
  RemoteHost,
  TailscaleStatus,
  FileEntry,
  SourceConnection,
  TaskSourceLink,
  ConnectorManifest,
  ConnectorActionDef
} from '../shared/types'

const api = {
  createTerminal: (payload: CreateTerminalPayload) =>
    ipcRenderer.invoke(IPC.TERMINAL_CREATE, payload),

  writeTerminal: (id: string, data: string) => ipcRenderer.send(IPC.TERMINAL_WRITE, { id, data }),

  resizeTerminal: (payload: ResizePayload) => ipcRenderer.send(IPC.TERMINAL_RESIZE, payload),

  killTerminal: (id: string) => ipcRenderer.invoke(IPC.TERMINAL_KILL, id),

  createShellTerminal: (cwd?: string): Promise<TerminalSession> =>
    ipcRenderer.invoke(IPC.SHELL_CREATE, cwd),

  onTerminalData: (callback: (event: { id: string; data: string }) => void) => {
    const listener = (_: Electron.IpcRendererEvent, event: { id: string; data: string }): void =>
      callback(event)
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

  loadConfig: (): Promise<AppConfig> => ipcRenderer.invoke(IPC.CONFIG_LOAD),

  saveConfig: (config: AppConfig) => ipcRenderer.invoke(IPC.CONFIG_SAVE, config),

  onConfigChanged: (callback: (config: AppConfig) => void) => {
    const listener = (_: Electron.IpcRendererEvent, config: AppConfig): void => callback(config)
    ipcRenderer.on(IPC.CONFIG_CHANGED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.CONFIG_CHANGED, listener)
    }
  },

  onMenuNewAgent: (callback: () => void) => {
    const listener = (): void => callback()
    ipcRenderer.on('menu:new-agent', listener)
    return () => {
      ipcRenderer.removeListener('menu:new-agent', listener)
    }
  },

  getPreviousSessions: () => ipcRenderer.invoke(IPC.SESSIONS_GET_PREVIOUS),

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

  removeWorktree: (projectPath: string, worktreePath: string, force?: boolean): Promise<boolean> =>
    ipcRenderer.invoke(IPC.GIT_REMOVE_WORKTREE, { projectPath, worktreePath, force }),

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
    }) => void
  ) => {
    const listener = (
      _: Electron.IpcRendererEvent,
      event: {
        workflowId: string
        connectorItem?: import('../../packages/shared/src/types').ConnectorItemContext
      }
    ): void => callback(event)
    ipcRenderer.on(IPC.SCHEDULER_EXECUTE, listener)
    return () => {
      ipcRenderer.removeListener(IPC.SCHEDULER_EXECUTE, listener)
    }
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
    status: 'success' | 'error'
    sessionsLaunched: number
    source?: 'scheduler' | 'manual'
  }): Promise<void> => ipcRenderer.invoke(IPC.WORKFLOW_EXECUTION_COMPLETE, data),

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

  // SSH
  testSshConnection: (
    host: RemoteHost
  ): Promise<{ success: boolean; message: string; durationMs: number }> =>
    ipcRenderer.invoke(IPC.SSH_TEST_CONNECTION, host),

  // Shell
  openExternal: (url: string): Promise<void> => ipcRenderer.invoke(IPC.OPEN_EXTERNAL, url),

  // App info
  getAppVersion: (): string => ipcRenderer.sendSync('get-app-version'),
  // Auto-update
  onUpdateDownloaded: (callback: (info: { version: string }) => void) => {
    const listener = (_: Electron.IpcRendererEvent, info: { version: string }): void =>
      callback(info)
    ipcRenderer.on(IPC.UPDATE_DOWNLOADED, listener)
    return () => {
      ipcRenderer.removeListener(IPC.UPDATE_DOWNLOADED, listener)
    }
  },
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
    >
  ): Promise<SourceConnection> => ipcRenderer.invoke(IPC.CONNECTION_CREATE, params),

  updateConnection: (
    id: string,
    updates: Partial<SourceConnection>
  ): Promise<SourceConnection | null> => ipcRenderer.invoke(IPC.CONNECTION_UPDATE, { id, updates }),

  deleteConnection: (id: string): Promise<void> => ipcRenderer.invoke(IPC.CONNECTION_DELETE, id),

  runWorkflowManual: (workflowId: string): Promise<void> =>
    ipcRenderer.invoke(IPC.WORKFLOW_RUN_MANUAL, { workflowId }),

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

  upsertTaskFromItem: (params: {
    connectionId: string
    item: import('../../packages/shared/src/types').ConnectorItemContext
    initialStatus: import('../../packages/shared/src/types').TaskStatus
    project?: string
  }): Promise<{ taskId: string; created: boolean }> =>
    ipcRenderer.invoke(IPC.CONNECTION_UPSERT_FROM_ITEM, params),

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

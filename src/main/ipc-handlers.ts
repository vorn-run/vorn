import { app, dialog, BrowserWindow, shell } from 'electron'
import { ipcMain } from 'electron'
import { safeHandle } from './ipc-safe-handle'
import { IPC, ResizePayload } from '../shared/types'
import type { ServerBridge } from './server/server-bridge'
import { registerCredentialHandlers, enrichPayloadWithCredentials } from './credential-handlers'

let bridge: ServerBridge | null = null

export function setBridge(b: ServerBridge): void {
  bridge = b
}

function requireBridge(): ServerBridge {
  if (!bridge) throw new Error('Server bridge not initialized')
  return bridge
}

export function registerIpcHandlers(): void {
  // ─── Delegated to server via bridge ────────────────────────────

  // Terminal (enriched with decrypted credentials when needed)
  safeHandle(IPC.TERMINAL_CREATE, async (_, payload) => {
    const enriched = await enrichPayloadWithCredentials(payload, requireBridge())
    return requireBridge().request(IPC.TERMINAL_CREATE, enriched)
  })
  safeHandle(IPC.TERMINAL_KILL, (_, id) => requireBridge().request(IPC.TERMINAL_KILL, id))
  safeHandle(IPC.SHELL_CREATE, (_, cwd) => requireBridge().request(IPC.SHELL_CREATE, cwd))

  // Config
  safeHandle(IPC.CONFIG_LOAD, () => requireBridge().request(IPC.CONFIG_LOAD))
  safeHandle(IPC.CONFIG_SAVE, (_, config) => requireBridge().request(IPC.CONFIG_SAVE, config))

  // Sessions
  safeHandle(IPC.SESSIONS_GET_PREVIOUS, () => requireBridge().request(IPC.SESSIONS_GET_PREVIOUS))
  safeHandle(IPC.SESSIONS_CLEAR, () => requireBridge().request(IPC.SESSIONS_CLEAR))
  safeHandle(IPC.SESSIONS_GET_RECENT, (_, projectPath) =>
    requireBridge().request(IPC.SESSIONS_GET_RECENT, projectPath)
  )
  safeHandle(IPC.TERMINAL_RENAME, (_, params) => requireBridge().request('terminal:rename', params))
  safeHandle(IPC.TERMINAL_REORDER, (_, ids) => requireBridge().request('terminal:reorder', ids))

  // Git
  safeHandle(IPC.GIT_IS_REPO, (_, projectPath) =>
    requireBridge().request(IPC.GIT_IS_REPO, projectPath)
  )
  safeHandle(IPC.GIT_LIST_BRANCHES, (_, projectPath) =>
    requireBridge().request(IPC.GIT_LIST_BRANCHES, projectPath)
  )
  safeHandle(IPC.GIT_LIST_REMOTE_BRANCHES, (_, projectPath) =>
    requireBridge().request(IPC.GIT_LIST_REMOTE_BRANCHES, projectPath)
  )
  safeHandle(IPC.GIT_CREATE_WORKTREE, (_, params) =>
    requireBridge().request(IPC.GIT_CREATE_WORKTREE, params)
  )
  safeHandle(IPC.GIT_REMOVE_WORKTREE, (_, params) =>
    requireBridge().request(IPC.GIT_REMOVE_WORKTREE, params)
  )
  safeHandle(IPC.GIT_RENAME_WORKTREE_BRANCH, (_, params) =>
    requireBridge().request(IPC.GIT_RENAME_WORKTREE_BRANCH, params)
  )
  safeHandle(IPC.GIT_RENAME_WORKTREE, (_, params) =>
    requireBridge().request(IPC.GIT_RENAME_WORKTREE, params)
  )
  safeHandle(IPC.GIT_WORKTREE_DIRTY, (_, path) =>
    requireBridge().request(IPC.GIT_WORKTREE_DIRTY, path)
  )
  safeHandle(IPC.GIT_LIST_WORKTREES, (_, projectPath) =>
    requireBridge().request(IPC.GIT_LIST_WORKTREES, projectPath)
  )
  safeHandle(IPC.GIT_CHECKOUT_BRANCH, (_, params) =>
    requireBridge().request(IPC.GIT_CHECKOUT_BRANCH, params)
  )
  safeHandle(IPC.GIT_GET_WORKTREE_BRANCH, (_, worktreePath) =>
    requireBridge().request(IPC.GIT_GET_WORKTREE_BRANCH, worktreePath)
  )
  safeHandle(IPC.WORKTREE_ACTIVE_SESSIONS, (_, worktreePath) =>
    requireBridge().request(IPC.WORKTREE_ACTIVE_SESSIONS, worktreePath)
  )
  safeHandle(IPC.GIT_GET_BRANCH, (_, cwd) => requireBridge().request(IPC.GIT_GET_BRANCH, cwd))
  safeHandle(IPC.GIT_DIFF_STAT, (_, cwd) => requireBridge().request(IPC.GIT_DIFF_STAT, cwd))
  safeHandle(IPC.GIT_DIFF_FULL, (_, cwd) => requireBridge().request(IPC.GIT_DIFF_FULL, cwd))
  safeHandle(IPC.GIT_COMMIT, (_, params) => requireBridge().request(IPC.GIT_COMMIT, params))
  safeHandle(IPC.GIT_PUSH, (_, cwd) => requireBridge().request(IPC.GIT_PUSH, cwd))

  // Scheduler
  safeHandle(IPC.SCHEDULER_GET_LOG, (_, workflowId) =>
    requireBridge().request(IPC.SCHEDULER_GET_LOG, workflowId)
  )
  safeHandle(IPC.SCHEDULER_GET_NEXT_RUN, (_, workflowId) =>
    requireBridge().request(IPC.SCHEDULER_GET_NEXT_RUN, workflowId)
  )

  // Task images
  safeHandle(IPC.TASK_IMAGE_SAVE, (_, params) =>
    requireBridge().request(IPC.TASK_IMAGE_SAVE, params)
  )
  safeHandle(IPC.TASK_IMAGE_DELETE, (_, params) =>
    requireBridge().request(IPC.TASK_IMAGE_DELETE, params)
  )
  safeHandle(IPC.TASK_IMAGE_GET_PATH, (_, params) =>
    requireBridge().request(IPC.TASK_IMAGE_GET_PATH, params)
  )
  safeHandle(IPC.TASK_IMAGE_CLEANUP, (_, taskId) =>
    requireBridge().request(IPC.TASK_IMAGE_CLEANUP, taskId)
  )

  // Headless sessions (enriched with decrypted credentials when needed)
  safeHandle(IPC.HEADLESS_CREATE, async (_, payload) => {
    const enriched = await enrichPayloadWithCredentials(payload, requireBridge())
    return requireBridge().request(IPC.HEADLESS_CREATE, enriched)
  })
  safeHandle(IPC.HEADLESS_KILL, (_, id) => requireBridge().request(IPC.HEADLESS_KILL, id))
  safeHandle(IPC.HEADLESS_LIST, () => requireBridge().request(IPC.HEADLESS_LIST))

  // Scripts
  safeHandle(IPC.SCRIPT_EXECUTE, (_, config) => requireBridge().request(IPC.SCRIPT_EXECUTE, config))

  // Workflow runs
  safeHandle(IPC.WORKFLOW_RUN_SAVE, (_, execution) =>
    requireBridge().request(IPC.WORKFLOW_RUN_SAVE, execution)
  )
  safeHandle(IPC.WORKFLOW_RUN_LIST, (_, workflowId, limit) =>
    requireBridge().request(IPC.WORKFLOW_RUN_LIST, { workflowId, limit })
  )
  safeHandle(IPC.WORKFLOW_RUN_LIST_BY_TASK, (_, taskId, limit) =>
    requireBridge().request(IPC.WORKFLOW_RUN_LIST_BY_TASK, { taskId, limit })
  )
  safeHandle(IPC.WORKFLOW_RUN_LIST_WAITING, () =>
    requireBridge().request(IPC.WORKFLOW_RUN_LIST_WAITING, {})
  )
  safeHandle(IPC.WORKFLOW_RUN_LIST_ALL, (_, workspaceId, limit) =>
    requireBridge().request(IPC.WORKFLOW_RUN_LIST_ALL, { workspaceId, limit })
  )
  safeHandle(IPC.WORKFLOW_RUN_LIST_RUNNING, () =>
    requireBridge().request(IPC.WORKFLOW_RUN_LIST_RUNNING, {})
  )

  // Session logs
  safeHandle(IPC.SESSION_LOG_LIST, (_, taskId) =>
    requireBridge().request(IPC.SESSION_LOG_LIST, { taskId })
  )
  safeHandle(IPC.SESSION_LOG_UPDATE, (_, entry) =>
    requireBridge().request(IPC.SESSION_LOG_UPDATE, entry)
  )

  // Session events
  safeHandle(IPC.SESSION_EVENT_LIST_BY_SESSION, (_, sessionId, limit) =>
    requireBridge().request(IPC.SESSION_EVENT_LIST_BY_SESSION, { sessionId, limit })
  )

  // Workflow execution complete
  safeHandle(IPC.WORKFLOW_EXECUTION_COMPLETE, (_, data) =>
    requireBridge().request(IPC.WORKFLOW_EXECUTION_COMPLETE, data)
  )

  // Agent / IDE detection
  safeHandle(IPC.IDE_DETECT, () => requireBridge().request(IPC.IDE_DETECT))
  safeHandle(IPC.AGENT_DETECT_INSTALLED, () => requireBridge().request(IPC.AGENT_DETECT_INSTALLED))
  safeHandle(IPC.IDE_OPEN, (_, params) => requireBridge().request(IPC.IDE_OPEN, params))

  // ─── Credential vault (requires safeStorage in main process) ───
  registerCredentialHandlers(requireBridge())

  // File explorer
  safeHandle(IPC.FILE_LIST_DIR, (_, dirPath) => requireBridge().request(IPC.FILE_LIST_DIR, dirPath))
  safeHandle(IPC.FILE_READ_CONTENT, (_, params) =>
    requireBridge().request(IPC.FILE_READ_CONTENT, params)
  )
  safeHandle(IPC.FILE_WRITE_CONTENT, (_, params) =>
    requireBridge().request(IPC.FILE_WRITE_CONTENT, params)
  )

  // Tailscale
  safeHandle(IPC.TAILSCALE_STATUS, () => requireBridge().request(IPC.TAILSCALE_STATUS))

  // SSH
  safeHandle(IPC.SSH_TEST_CONNECTION, (_, host) =>
    requireBridge().request(IPC.SSH_TEST_CONNECTION, host)
  )

  // Connectors
  safeHandle(IPC.CONNECTOR_LIST, () => requireBridge().request(IPC.CONNECTOR_LIST))
  safeHandle(IPC.CONNECTOR_GET, (_, id) => requireBridge().request(IPC.CONNECTOR_GET, id))
  safeHandle(IPC.CONNECTION_LIST, (_, params) =>
    requireBridge().request(IPC.CONNECTION_LIST, params)
  )
  safeHandle(IPC.CONNECTION_CREATE, (_, params) =>
    requireBridge().request(IPC.CONNECTION_CREATE, params)
  )
  safeHandle(IPC.CONNECTION_UPDATE, (_, params) =>
    requireBridge().request(IPC.CONNECTION_UPDATE, params)
  )
  safeHandle(IPC.CONNECTION_DELETE, (_, id) => requireBridge().request(IPC.CONNECTION_DELETE, id))
  safeHandle(IPC.CONNECTION_UPSERT_FROM_ITEM, (_, params) =>
    requireBridge().request(IPC.CONNECTION_UPSERT_FROM_ITEM, params)
  )
  safeHandle(IPC.WORKFLOW_RUN_MANUAL, (_, params) =>
    requireBridge().request(IPC.WORKFLOW_RUN_MANUAL, params)
  )
  safeHandle(IPC.CONNECTION_BACKFILL, (_, params) =>
    requireBridge().request(IPC.CONNECTION_BACKFILL, params)
  )
  safeHandle(IPC.CREDENTIALS_SET_DECRYPTED, (_, params) =>
    requireBridge().request(IPC.CREDENTIALS_SET_DECRYPTED, params)
  )
  safeHandle(IPC.CREDENTIALS_CLEAR_DECRYPTED, (_, params) =>
    requireBridge().request(IPC.CREDENTIALS_CLEAR_DECRYPTED, params)
  )
  safeHandle(IPC.CONNECTION_EXECUTE_ACTION, (_, params) =>
    requireBridge().request(IPC.CONNECTION_EXECUTE_ACTION, params)
  )
  safeHandle(IPC.CONNECTION_LIST_ACTIONS, (_, connectionId) =>
    requireBridge().request(IPC.CONNECTION_LIST_ACTIONS, connectionId)
  )
  safeHandle(IPC.CONNECTION_LIST_MCP_TOOLS, (_, connectionId) =>
    requireBridge().request(IPC.CONNECTION_LIST_MCP_TOOLS, connectionId)
  )
  safeHandle(IPC.CONNECTION_REFRESH_MCP_TOOLS, (_, connectionId) =>
    requireBridge().request(IPC.CONNECTION_REFRESH_MCP_TOOLS, connectionId)
  )
  safeHandle(IPC.CONNECTION_GET_SOURCE_LINK, (_, taskId) =>
    requireBridge().request(IPC.CONNECTION_GET_SOURCE_LINK, taskId)
  )
  safeHandle(IPC.CONNECTOR_DETECT_REPO, (_, projectPath) =>
    requireBridge().request(IPC.CONNECTOR_DETECT_REPO, projectPath)
  )
  safeHandle(IPC.CONNECTOR_SEED_WORKFLOW, (_, params) =>
    requireBridge().request(IPC.CONNECTOR_SEED_WORKFLOW, params)
  )
  safeHandle(IPC.CONNECTOR_STATUS, () => requireBridge().request(IPC.CONNECTOR_STATUS))

  // ─── Electron-only handlers (stay local) ───────────────────────

  safeHandle(IPC.DIALOG_OPEN_DIRECTORY, async (event) => {
    const win = BrowserWindow.fromWebContents(event.sender)
    if (!win) return null
    const result = await dialog.showOpenDialog(win, {
      properties: ['openDirectory'],
      title: 'Select Project Folder'
    })
    if (result.canceled || result.filePaths.length === 0) return null
    return result.filePaths[0]
  })

  safeHandle(IPC.DIALOG_OPEN_FILE, async (event) => {
    const win = BrowserWindow.fromWebContents(event.sender)
    if (!win) return null
    const result = await dialog.showOpenDialog(win, {
      properties: ['openFile'],
      title: 'Select SSH Key'
    })
    if (result.canceled || result.filePaths.length === 0) return null
    return result.filePaths[0]
  })

  safeHandle(IPC.DIALOG_OPEN_IMAGE, async () => {
    const win = BrowserWindow.getFocusedWindow()
    if (!win) return null
    const result = await dialog.showOpenDialog(win, {
      properties: ['openFile', 'multiSelections'],
      filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg'] }]
    })
    return result.canceled ? null : result.filePaths
  })

  // App version (sync)
  ipcMain.on('get-app-version', (event) => {
    event.returnValue = app.getVersion()
  })

  // Open external URL in default browser (only http/https)
  safeHandle(IPC.OPEN_EXTERNAL, (_, rawUrl: string) => {
    if (typeof rawUrl !== 'string') throw new Error('Invalid URL: expected string')
    let parsed: URL
    try {
      parsed = new URL(rawUrl.trim())
    } catch {
      throw new Error('Invalid URL: parse failure')
    }
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      throw new Error('Invalid URL protocol')
    }
    return shell.openExternal(parsed.toString())
  })

  // ─── Fire-and-forget → bridge notifications ────────────────────

  ipcMain.on(IPC.TERMINAL_WRITE, (_, { id, data }: { id: string; data: string }) =>
    bridge?.notify(IPC.TERMINAL_WRITE, { id, data })
  )

  ipcMain.on(IPC.TERMINAL_RESIZE, (_, payload: ResizePayload) =>
    bridge?.notify(IPC.TERMINAL_RESIZE, payload)
  )
}

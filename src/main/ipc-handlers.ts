import { app, dialog, BrowserWindow, session, shell } from 'electron'
import { ipcMain } from 'electron'
import { safeHandle } from './ipc-safe-handle'
import { IPC, ResizePayload, browserPartition } from '../shared/types'
import type { ServerBridge } from './server/server-bridge'
import type { RequestMethods } from '@vornrun/shared/protocol'
import * as browserRegistry from './browser-registry'
import { setFileRoot, allowsFileUrl } from './browser-file-scope'
import { watchArtifact, stopWatching } from './artifact-watcher'
import * as deviceRegistry from './device-registry'
import { registerCredentialHandlers, enrichPayloadWithCredentials } from './credential-handlers'
import log from './logger'

let bridge: ServerBridge | null = null

export function setBridge(b: ServerBridge): void {
  bridge = b
  registerInboundHandlers(b)
}

/**
 * Answer the `browser:*` methods the server relays to us.
 *
 * These flow the opposite way from everything else in this file: an agent calls
 * an MCP tool, the MCP server asks the Vorn server, and the Vorn server asks
 * *us*, because the guest `<webview>` and its CDP debugger only exist in this
 * process. Params arrive already carrying the session id the MCP layer resolved
 * from `VORN_SESSION_ID`.
 */
function registerInboundHandlers(b: ServerBridge): void {
  type P<M extends keyof RequestMethods> = RequestMethods[M]['params']
  b.handle('browser:readPage', (p) => browserRegistry.readPage(p as P<'browser:readPage'>))
  b.handle('browser:getText', (p) => browserRegistry.getText(p as P<'browser:getText'>))
  b.handle('browser:consoleMessages', (p) =>
    browserRegistry.consoleMessages(p as P<'browser:consoleMessages'>)
  )
  b.handle('browser:networkRequests', (p) =>
    browserRegistry.networkRequests(p as P<'browser:networkRequests'>)
  )
  b.handle('browser:screenshot', (p) => browserRegistry.screenshot(p as P<'browser:screenshot'>))
  b.handle('browser:interact', (p) => browserRegistry.interact(p as P<'browser:interact'>))
  b.handle('browser:tabs', (p) => browserRegistry.tabs(p as P<'browser:tabs'>))
  b.handle('browser:openPane', (p) => browserRegistry.openPane(p as P<'browser:openPane'>))
  b.handle('browser:navigate', (p) => browserRegistry.navigate(p as P<'browser:navigate'>))
  b.handle('browser:history', (p) => browserRegistry.goHistory(p as P<'browser:history'>))
  b.handle('browser:listTabs', (p) => browserRegistry.listTabs(p as P<'browser:listTabs'>))
  b.handle('browser:find', (p) => browserRegistry.find(p as P<'browser:find'>))

  // The device family answers here for a sharper version of the same reason:
  // the simulator is driven by a child `idb_companion` process over a unix
  // socket, and only this process owns it.
  b.handle('device:list', () => deviceRegistry.listDevices())
  b.handle('device:claim', (p) => deviceRegistry.claim(p as P<'device:claim'>))
  b.handle('device:release', (p) => deviceRegistry.release(p as P<'device:release'>))
  b.handle('device:readScreen', (p) => deviceRegistry.readScreen(p as P<'device:readScreen'>))
  b.handle('device:find', (p) => deviceRegistry.findElements(p as P<'device:find'>))
  b.handle('device:interact', (p) => deviceRegistry.interact(p as P<'device:interact'>))
  b.handle('device:screenshot', (p) => deviceRegistry.screenshot(p as P<'device:screenshot'>))
  b.handle('device:launch', (p) => deviceRegistry.launch(p as P<'device:launch'>))
  b.handle('device:terminate', (p) => deviceRegistry.terminate(p as P<'device:terminate'>))
  b.handle('device:install', (p) => deviceRegistry.install(p as P<'device:install'>))
  b.handle('device:openUrl', (p) => deviceRegistry.openUrl(p as P<'device:openUrl'>))
  b.handle('device:logs', (p) => deviceRegistry.logsFor(p as P<'device:logs'>))
  b.handle('device:openPane', (p) => deviceRegistry.openPane(p as P<'device:openPane'>))
}

function requireBridge(): ServerBridge {
  if (!bridge) throw new Error('Server bridge not initialized')
  return bridge
}

/** Sessions whose partition already carries the filter, so it is installed once. */
const guardedPartitions = new Set<string>()

/**
 * Refuse every file request this session's pane is not entitled to make.
 *
 * Checking the url an agent navigates to is not the guard — it is only the
 * first of many requests. Once a guest is on a file page, its own scripts can
 * `fetch`, `iframe` or `XHR` any other path, and none of those pass through
 * `navigate`. Without a filter here the scoping would be decorative: one hop
 * inside the root and the whole disk is readable again.
 *
 * Installed on the session partition, which is already per session
 * (`persist:vorn-browser-<id>`), so the root it enforces is the right one.
 */
function guardFileRequests(sessionId: string): void {
  const partition = browserPartition(sessionId)
  if (guardedPartitions.has(partition)) return
  guardedPartitions.add(partition)

  session.fromPartition(partition).webRequest.onBeforeRequest((details, callback) => {
    if (!details.url.startsWith('file:')) return callback({})
    const allowed = allowsFileUrl(sessionId, details.url)
    if (!allowed) {
      // The path is deliberately not logged. A refused one is by definition
      // outside the project — a home directory, a key, whatever the page asked
      // for — and `main.log` outlives the session and travels with a bug
      // report. The refusal is the fact worth keeping; `debug` carries the
      // path for anyone actually chasing one.
      log.warn(`[browser] refused a file request outside session ${sessionId}`)
      log.debug(`[browser] refused url for session ${sessionId}: ${details.url}`)
    }
    callback({ cancel: !allowed })
  })
}

export function registerIpcHandlers(): void {
  // ─── Delegated to server via bridge ────────────────────────────

  // Terminal (enriched with decrypted credentials when needed)
  safeHandle(IPC.TERMINAL_CREATE, async (_, payload) => {
    const enriched = await enrichPayloadWithCredentials(payload, requireBridge())
    return requireBridge().request(IPC.TERMINAL_CREATE, enriched)
  })
  safeHandle(IPC.TERMINAL_KILL, (_, id) => {
    // Killing the session hands its device back. Nothing else does: closing the
    // pane releases, but a session killed with the pane already shut — or never
    // opened, because an agent claimed it headlessly — would otherwise strand a
    // running companion and a claim no session owns, locking that simulator out
    // of every other session until the app quits.
    deviceRegistry.releaseForSession(id as string)
    return requireBridge().request(IPC.TERMINAL_KILL, id)
  })
  safeHandle(IPC.SHELL_CREATE, (_, cwd) => requireBridge().request(IPC.SHELL_CREATE, cwd))

  // Config
  safeHandle(IPC.CONFIG_LOAD, () => requireBridge().request(IPC.CONFIG_LOAD))
  safeHandle(IPC.CONFIG_SAVE, (_, config) => requireBridge().request(IPC.CONFIG_SAVE, config))

  // Sessions
  safeHandle(IPC.TERMINAL_ATTACH, (_, id) => requireBridge().request(IPC.TERMINAL_ATTACH, { id }))
  safeHandle(IPC.TERMINAL_LIST_ACTIVE, () => requireBridge().request(IPC.TERMINAL_LIST_ACTIVE))
  safeHandle(IPC.SESSIONS_RESTORED, () => requireBridge().request(IPC.SESSIONS_RESTORED))
  safeHandle(IPC.SESSIONS_RESUME, (_, params) =>
    requireBridge().request(IPC.SESSIONS_RESUME, params)
  )
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
  safeHandle(IPC.WORKTREE_INVENTORY, (_, params) =>
    requireBridge().request(IPC.WORKTREE_INVENTORY, params)
  )
  safeHandle(IPC.WORKTREE_RECLAIM_ARTIFACTS, (_, params) =>
    requireBridge().request(IPC.WORKTREE_RECLAIM_ARTIFACTS, params)
  )
  safeHandle(IPC.WORKTREE_REMOVE_MANY, (_, params) =>
    requireBridge().request(IPC.WORKTREE_REMOVE_MANY, params)
  )
  safeHandle(IPC.WORKTREE_PRUNE_ORPHANS, (_, params) =>
    requireBridge().request(IPC.WORKTREE_PRUNE_ORPHANS, params)
  )
  safeHandle(IPC.GIT_DELETE_BRANCHES, (_, params) =>
    requireBridge().request(IPC.GIT_DELETE_BRANCHES, params)
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
  safeHandle(IPC.WORKFLOW_RUN_CLAIM, (_, req) =>
    requireBridge().request(IPC.WORKFLOW_RUN_CLAIM, req)
  )
  safeHandle(IPC.WORKFLOW_RUN_RELEASE, (_, req) =>
    requireBridge().request(IPC.WORKFLOW_RUN_RELEASE, req)
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
  safeHandle(IPC.PROJECT_DETECT_MOBILE, (_, params) =>
    requireBridge().request(IPC.PROJECT_DETECT_MOBILE, params)
  )
  safeHandle(IPC.AGENT_DETECT_INSTALLED, () => requireBridge().request(IPC.AGENT_DETECT_INSTALLED))
  safeHandle(IPC.IDE_OPEN, (_, params) => requireBridge().request(IPC.IDE_OPEN, params))

  // ─── Credential vault (requires safeStorage in main process) ───
  registerCredentialHandlers(requireBridge())

  // File explorer
  safeHandle(IPC.FILE_LIST_DIR, (_, dirPath) => requireBridge().request(IPC.FILE_LIST_DIR, dirPath))
  safeHandle(IPC.SHELL_LIST_EXECUTABLES, () => requireBridge().request(IPC.SHELL_LIST_EXECUTABLES))
  safeHandle(IPC.SHELL_LIST_INSTALLED, () => requireBridge().request(IPC.SHELL_LIST_INSTALLED))
  safeHandle(IPC.FILE_READ_CONTENT, (_, params) =>
    requireBridge().request(IPC.FILE_READ_CONTENT, params)
  )
  safeHandle(IPC.FILE_WRITE_CONTENT, (_, params) =>
    requireBridge().request(IPC.FILE_WRITE_CONTENT, params)
  )

  // Tailscale
  safeHandle(IPC.TAILSCALE_STATUS, () => requireBridge().request(IPC.TAILSCALE_STATUS))
  safeHandle(IPC.SERVER_REACHABLE_URLS, () => requireBridge().request(IPC.SERVER_REACHABLE_URLS))
  safeHandle(IPC.PAIRING_START, () => requireBridge().request(IPC.PAIRING_START))
  safeHandle(IPC.PAIRING_PENDING, () => requireBridge().request(IPC.PAIRING_PENDING))
  safeHandle(IPC.PAIRING_APPROVE, (_, p) => requireBridge().request(IPC.PAIRING_APPROVE, p))
  safeHandle(IPC.PAIRING_DENY, (_, p) => requireBridge().request(IPC.PAIRING_DENY, p))
  safeHandle(IPC.PAIRING_CANCEL, () => requireBridge().request(IPC.PAIRING_CANCEL))
  safeHandle(IPC.TOKEN_LIST, () => requireBridge().request(IPC.TOKEN_LIST))
  safeHandle(IPC.TOKEN_CREATE, (_, params) => requireBridge().request(IPC.TOKEN_CREATE, params))
  safeHandle(IPC.TOKEN_REVOKE, (_, id) => requireBridge().request(IPC.TOKEN_REVOKE, id))

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
  safeHandle(IPC.CONNECTOR_INBOX_COMPLETE, (_, params) =>
    requireBridge().request(IPC.CONNECTOR_INBOX_COMPLETE, params)
  )
  safeHandle(IPC.CONNECTOR_INBOX_RENEW, (_, params) =>
    requireBridge().request(IPC.CONNECTOR_INBOX_RENEW, params)
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
  safeHandle(IPC.WEBHOOK_INFO, () => requireBridge().request(IPC.WEBHOOK_INFO, undefined))
  safeHandle(IPC.CONNECTION_LIST_ACTIONS, (_, connectionId) =>
    requireBridge().request(IPC.CONNECTION_LIST_ACTIONS, connectionId)
  )
  safeHandle(IPC.CONNECTION_LIST_MCP_TOOLS, (_, connectionId) =>
    requireBridge().request(IPC.CONNECTION_LIST_MCP_TOOLS, connectionId)
  )
  safeHandle(IPC.CONNECTION_REFRESH_MCP_TOOLS, (_, connectionId) =>
    requireBridge().request(IPC.CONNECTION_REFRESH_MCP_TOOLS, connectionId)
  )
  safeHandle(IPC.CONNECTOR_PROBE_SDK, (_, request) =>
    requireBridge().request(IPC.CONNECTOR_PROBE_SDK, request)
  )
  safeHandle(IPC.CONNECTOR_CATALOG, () => requireBridge().request(IPC.CONNECTOR_CATALOG))
  safeHandle(IPC.CONNECTOR_CATALOG_REFRESH, () =>
    requireBridge().request(IPC.CONNECTOR_CATALOG_REFRESH)
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

  // ─── Browser pane (Electron-only: guests live here) ────────────
  //
  // A `<webview>` carries no session identity — only a partition string — so
  // the renderer tells us which guest belongs to which session as soon as it
  // attaches. Everything the agent does to the pane keys off that mapping.
  ipcMain.on(IPC.BROWSER_ATTACH, (_, { sessionId, webContentsId, fileRoot }) => {
    browserRegistry.attach(sessionId, webContentsId)
    // The pane's reach into the disk, which only the renderer knows: the
    // session's worktree if it has one, else its project. Set after attach so a
    // pane that failed to attach is left with no root rather than a live one.
    setFileRoot(sessionId, fileRoot)
    guardFileRequests(sessionId)
  })
  ipcMain.on(IPC.BROWSER_DETACH, (_, sessionId: string) => {
    browserRegistry.detach(sessionId)
    setFileRoot(sessionId, undefined)
    // The watcher outlives nothing. A pane that closed has no design showing,
    // and a descriptor left open would report changes to a session that is gone.
    stopWatching(sessionId)
  })
  ipcMain.on(
    IPC.BROWSER_WATCH_FILE,
    (_, { sessionId, path }: { sessionId: string; path: string | null }) => {
      watchArtifact(sessionId, path)
    }
  )
  ipcMain.on(IPC.BROWSER_TABS_CHANGED, (_, { sessionId, tabs }) => {
    browserRegistry.syncTabs(sessionId, tabs)
  })

  // The picker is user-initiated and answers back to the renderer that armed
  // it, not to an agent. A cancel resolves as `null` rather than an error: the
  // person changing their mind is an ordinary outcome, not a failure.
  // What the loaded page declares itself to be. The renderer draws the pane's
  // chrome from this, and only main can ask — a `<webview>` guest is reachable
  // from here and nowhere else.
  safeHandle(IPC.BROWSER_READ_MANIFEST, async (_, sessionId: string) =>
    browserRegistry.readManifest({ sessionId })
  )
  safeHandle(
    IPC.BROWSER_SET_TWEAK,
    async (_, { sessionId, key, value }: { sessionId: string; key: string; value: unknown }) =>
      browserRegistry.setTweak({ sessionId, key, value })
  )
  safeHandle(IPC.BROWSER_PICK_START, async (_, sessionId: string) => {
    try {
      return await browserRegistry.startPick({ sessionId })
    } catch (err) {
      if (err instanceof Error && err.message === 'Selection cancelled') return null
      throw err
    }
  })
  ipcMain.on(IPC.BROWSER_PICK_CANCEL, (_, sessionId: string) => {
    browserRegistry.cancelPick(sessionId)
  })
  safeHandle(IPC.BROWSER_ANNOTATE, (_, params) => browserRegistry.annotate(params))

  // ─── Device pane (Electron-only: the companion lives here) ─────
  //
  // The browser pane's guest renders itself; a simulator's does not. There is
  // no `<webview>` equivalent, so the pane is a still image polled from main
  // and every touch is relayed back the same way. These are the person's
  // channel into the same registry the agent's tools use — same claim, same
  // generation counter, so a person tapping the pane invalidates the agent's
  // refs exactly as the agent's own tap would.
  safeHandle(IPC.DEVICE_SCREENSHOT, (_, params) => deviceRegistry.screenshot(params))
  safeHandle(IPC.DEVICE_INTERACT, (_, params) => deviceRegistry.interact(params))
  safeHandle(IPC.DEVICE_LIST, () => deviceRegistry.listDevices())
  safeHandle(IPC.DEVICE_CLAIM, (_, params) => deviceRegistry.claim(params))
  safeHandle(IPC.DEVICE_RELEASE, (_, params) => deviceRegistry.release(params))
  // Both are read-only by design: pointing at or drawing on the screen must
  // never move it, or the person ends up describing an element the agent then
  // finds gone.
  safeHandle(IPC.DEVICE_PICKED, (_, params) => deviceRegistry.pickAt(params))
  safeHandle(IPC.DEVICE_ANNOTATE, (_, params) => deviceRegistry.annotate(params))

  // ─── Fire-and-forget → bridge notifications ────────────────────

  ipcMain.on(IPC.TERMINAL_WRITE, (_, { id, data }: { id: string; data: string }) =>
    bridge?.notify(IPC.TERMINAL_WRITE, { id, data })
  )

  ipcMain.on(IPC.TERMINAL_RESIZE, (_, payload: ResizePayload) =>
    bridge?.notify(IPC.TERMINAL_RESIZE, payload)
  )
}

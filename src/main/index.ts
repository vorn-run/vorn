import { app, BrowserWindow, ipcMain, nativeImage, screen, globalShortcut } from 'electron'
import path from 'node:path'
import { registerIpcHandlers, setBridge } from './ipc-handlers'
import * as browserRegistry from './browser-registry'
import * as deviceRegistry from './device-registry'
import { installCompanionQuitHook } from './device-companion'
import { installConnectorCredentialsSync } from './connector-credentials-sync'
import { createMenu } from './menu'
import { updateManager } from './update-manager'
import { IPC, PermissionRequestInfo } from '../shared/types'
import { setArtifactNotify } from './artifact-watcher'
import { SURFACE } from '../shared/surface'
import { launchServer, stopServer, getServerBridge } from './server/server-launcher'
import { readHostSettings } from './server/host-store'
import { registerConnectHandlers, showConnectWindow } from './server/connect-window'
import type { ServerBridge } from './server/server-bridge'
import log from './logger'

let isQuitting = false

// Ensure only one instance of the app runs at a time.
// Without this, spawning bugs (e.g. using process.execPath to launch the server)
// could cause an infinite cascade of Electron app instances.
// In dev mode, skip the lock and isolate userData so dev and production
// don't clobber each other's config/DB.
const isDev = !!process.env.ELECTRON_RENDERER_URL
if (isDev) {
  app.setPath('userData', path.join(app.getPath('userData'), '-dev'))
} else {
  const gotTheLock = app.requestSingleInstanceLock()
  if (!gotTheLock) {
    app.quit()
  }
}

// Prevent EPIPE and other uncaught errors from crashing the main process
process.on('uncaughtException', (err) => {
  if ((err as NodeJS.ErrnoException).code === 'EPIPE') return
  log.error('[main] uncaughtException:', err)
})

process.on('unhandledRejection', (reason) => {
  log.error('[main] unhandledRejection:', reason)
})

let mainWindow: BrowserWindow | null = null
let widgetWindow: BrowserWindow | null = null

function createWindow(): void {
  const isMac = process.platform === 'darwin'

  mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    minWidth: 800,
    minHeight: 600,
    show: false,
    icon: path.join(__dirname, '../../resources/icon.png'),
    titleBarStyle: isMac ? 'hiddenInset' : 'hidden',
    frame: false,
    ...(isMac
      ? {
          trafficLightPosition: { x: 16, y: 13 }
        }
      : {}),
    backgroundColor: SURFACE.base,
    webPreferences: {
      preload: path.join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
      // Session browser panes render remote pages in a <webview>, which is a
      // separate process with its own sandbox — the renderer never runs that
      // content itself.
      webviewTag: true
    }
  })

  mainWindow.once('ready-to-show', () => {
    mainWindow?.maximize()
    mainWindow?.show()
  })

  mainWindow.on('maximize', () => {
    mainWindow?.webContents.send(IPC.WINDOW_MAXIMIZED_CHANGED, true)
  })
  mainWindow.on('unmaximize', () => {
    mainWindow?.webContents.send(IPC.WINDOW_MAXIMIZED_CHANGED, false)
  })

  // Set dock icon on macOS (needed in dev mode since there's no app bundle)
  if (process.platform === 'darwin') {
    const iconPath = path.join(__dirname, '../../resources/icon.png')
    app.dock?.setIcon(nativeImage.createFromPath(iconPath))
  }

  if (process.env.ELECTRON_RENDERER_URL) {
    mainWindow.loadURL(process.env.ELECTRON_RENDERER_URL)
  } else {
    mainWindow.loadFile(path.join(__dirname, '../renderer/index.html'))
  }

  // macOS: close hides window, app stays alive (quit via Cmd+Q)
  mainWindow.on('close', (e) => {
    if (process.platform === 'darwin' && !isQuitting) {
      e.preventDefault()
      mainWindow?.hide()
      showWidget()
      app.dock?.show().catch(() => {})
      return
    }
  })

  mainWindow.on('closed', () => {
    mainWindow = null
    if (widgetWindow && !widgetWindow.isDestroyed()) {
      widgetWindow.destroy()
      widgetWindow = null
    }
  })
}

let widgetEnabled = true
let widgetReady = false

function sendToWidget(channel: string, ...args: unknown[]): void {
  if (!widgetWindow || widgetWindow.isDestroyed() || !widgetReady) return
  widgetWindow.webContents.send(channel, ...args)
}

function createWidgetWindow(): void {
  if (widgetWindow && !widgetWindow.isDestroyed()) return

  const isMac = process.platform === 'darwin'
  const display = screen.getPrimaryDisplay()
  const { width: screenW, height: screenH } = display.workAreaSize
  const widgetW = 280
  const widgetH = 400

  widgetWindow = new BrowserWindow({
    width: widgetW,
    height: widgetH,
    x: screenW - widgetW - 20,
    y: screenH - widgetH - 20,
    frame: false,
    transparent: true,
    alwaysOnTop: true,
    skipTaskbar: true,
    resizable: false,
    hasShadow: false,
    show: false,
    backgroundColor: '#00000000',
    ...(isMac ? { type: 'panel' } : {}),
    webPreferences: {
      preload: path.join(__dirname, '../preload/widget-preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false
    }
  })

  if (isMac) {
    widgetWindow.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true })
  }

  if (process.env.ELECTRON_RENDERER_URL) {
    widgetWindow.loadURL(process.env.ELECTRON_RENDERER_URL + '/widget.html')
  } else {
    widgetWindow.loadFile(path.join(__dirname, '../renderer/widget.html'))
  }

  widgetReady = false
  widgetWindow.webContents.once('did-finish-load', () => {
    widgetReady = true
    // Push current sessions to widget on load
    const b = getServerBridge()
    if (b) {
      b.request<import('../shared/types').TerminalSession[]>('terminal:listActive')
        .then((sessions) => {
          const agents = sessions.map((s) => ({
            id: s.id,
            agentType: s.agentType,
            displayName: s.displayName,
            projectName: s.projectName,
            status: s.status
          }))
          sendToWidget(IPC.WIDGET_STATUS_UPDATE, agents)
        })
        .catch(() => {})
    }
  })
  widgetWindow.on('closed', () => {
    widgetWindow = null
    widgetReady = false
  })
}

function showWidget(): void {
  if (!widgetEnabled) return
  if (!widgetWindow || widgetWindow.isDestroyed()) {
    createWidgetWindow()
  }
  if (widgetWindow && !widgetWindow.isVisible()) {
    widgetWindow.showInactive()
  }
}

function hideWidget(): void {
  if (widgetWindow && !widgetWindow.isDestroyed() && widgetWindow.isVisible()) {
    widgetWindow.hide()
  }
}

function toggleWidget(): void {
  if (!widgetWindow || widgetWindow.isDestroyed() || !widgetWindow.isVisible()) {
    widgetEnabled = true
    showWidget()
  } else {
    hideWidget()
  }
}

/**
 * Wire up server notification forwarding.
 * When the server pushes events via WebSocket, forward them to the
 * renderer and widget windows.
 */
function wireServerNotifications(bridge: ServerBridge): void {
  bridge.on('server-notification', (method: string, params: unknown) => {
    switch (method) {
      // Terminal data/exit → forward to renderer
      case IPC.TERMINAL_DATA:
      case IPC.TERMINAL_EXIT:
      case IPC.HEADLESS_DATA:
      case IPC.HEADLESS_EXIT:
      case IPC.SCRIPT_DATA:
      case IPC.SCRIPT_EXIT:
      case IPC.WORKTREE_CONFIRM_CLEANUP:
      case IPC.SESSION_CREATED:
      case IPC.SESSION_UPDATED:
      case IPC.SESSION_REORDERED:
      case IPC.CONFIG_CHANGED:
      case IPC.SCHEDULER_EXECUTE:
      case IPC.SCHEDULER_STOP_RUN:
      case IPC.SCHEDULER_MISSED:
      case IPC.WORKFLOW_EXECUTION_COMPLETE:
      case IPC.WORKFLOW_GATE_RESOLVED:
        if (mainWindow && !mainWindow.isDestroyed()) {
          mainWindow.webContents.send(method, params)
        }
        break

      // Widget status updates
      case IPC.WIDGET_STATUS_UPDATE:
        sendToWidget(method, params)
        break

      // Permission requests → both widget and main window
      case IPC.WIDGET_PERMISSION_REQUEST: {
        const permReq = params as PermissionRequestInfo
        sendToWidget(method, permReq)
        if (mainWindow && !mainWindow.isDestroyed()) {
          mainWindow.webContents.send(method, permReq)
        }
        // Show widget only when main window is not focused
        if (!mainWindow || mainWindow.isDestroyed() || !mainWindow.isFocused()) {
          showWidget()
        }
        updatePermissionShortcuts()
        break
      }

      case IPC.WIDGET_PERMISSION_CANCELLED:
        sendToWidget(method, params)
        updatePermissionShortcuts()
        break

      default:
        // Forward any other server notifications to renderer
        if (mainWindow && !mainWindow.isDestroyed()) {
          mainWindow.webContents.send(method, params)
        }
        break
    }
  })
}

function updatePermissionShortcuts(): void {
  const bridge = getServerBridge()
  if (!bridge) return

  // Unregister old shortcuts
  globalShortcut.unregister('CmdOrCtrl+Shift+A')
  globalShortcut.unregister('CmdOrCtrl+Shift+D')

  // We can't query pending permissions synchronously anymore since they're on the server.
  // Instead, register shortcuts that send approval/denial for whatever is pending.
  // The server tracks what's pending.
  globalShortcut.register('CmdOrCtrl+Shift+A', () => {
    bridge.request('permission:resolve-top', { allow: true }).catch(() => {})
  })
  globalShortcut.register('CmdOrCtrl+Shift+D', () => {
    bridge.request('permission:resolve-top', { allow: false }).catch(() => {})
  })
}

// When a second instance is launched, focus the existing window instead
app.on('second-instance', () => {
  if (mainWindow) {
    if (mainWindow.isMinimized()) mainWindow.restore()
    mainWindow.show()
    mainWindow.focus()
  }
})

/**
 * Guards for the pages a session's browser pane loads.
 *
 * `webviewTag` lets the renderer embed arbitrary remote content, so each guest
 * gets locked down as it attaches: no node integration, no popping out into new
 * windows, and no privileged permissions granted by default. A page opening a
 * window becomes a navigation in the same pane instead, which is what a user
 * expects from a pane that shows one url at a time.
 */
function hardenWebviews(): void {
  app.on('web-contents-created', (_event, contents) => {
    // Strip anything privileged off a guest before it attaches.
    contents.on('will-attach-webview', (_e, webPreferences) => {
      delete webPreferences.preload
      webPreferences.nodeIntegration = false
      webPreferences.contextIsolation = true
    })

    if (contents.getType() !== 'webview') return

    // A page asking for a new window navigates this pane instead — the pane
    // shows one url at a time, and popping out a real BrowserWindow would
    // escape every guard set here.
    contents.setWindowOpenHandler(({ url }) => {
      try {
        if (/^https?:$/.test(new URL(url).protocol)) {
          void contents.loadURL(url).catch(() => {})
        }
      } catch {
        /* unparseable url — deny */
      }
      return { action: 'deny' }
    })

    contents.session.setPermissionRequestHandler((_wc, permission, callback) => {
      // Camera, mic, geolocation and clipboard reads are not things an embedded
      // preview needs; allowlist more only if something real requires it.
      callback(permission === 'fullscreen')
    })
  })
}

app.whenReady().then(async () => {
  hardenWebviews()
  // Before launchServer, unlike everything else: these are what let someone
  // correct an unreachable host, so they cannot depend on reaching one.
  registerConnectHandlers()
  let bridge: ServerBridge
  try {
    bridge = await launchServer()
  } catch (err) {
    log.error('[main] Failed to launch server:', err)
    // A local server that will not start is fatal — there is nothing to show and
    // nothing to retry. A host that will not answer is ordinary: the machine is
    // asleep, the wifi changed, someone is on a train. Quitting there means the
    // app vanishes on launch with no way back, and no way to correct a mistyped
    // address, so host mode gets a window that explains itself instead.
    if (readHostSettings().mode === 'host') {
      showConnectWindow(err instanceof Error ? err.message : String(err))
      return
    }
    app.quit()
    return
  }

  setBridge(bridge)
  // Lets the browser and device registries ask the renderer for a pane, so an
  // agent can open one itself instead of waiting on a human click. Both are
  // wired from the same helper: a registry left unwired keeps its default
  // no-op `sendToRenderer`, which makes `openPane` return success while the
  // renderer is never told — the pane simply never appears, with nothing to
  // say why.
  const paneSend =
    (surface: string) =>
    (channel: string, params: unknown): void => {
      // Throwing rather than dropping: a swallowed send leaves the caller to
      // burn its attach timeout and then report "the pane did not open in
      // time", when the truth is there was no window to open one in.
      if (!mainWindow || mainWindow.isDestroyed()) {
        throw new Error(`The Vorn window is not open, so a ${surface} pane cannot be shown.`)
      }
      mainWindow.webContents.send(channel, params)
    }
  browserRegistry.setRendererSend(paneSend('browser'))
  deviceRegistry.setRendererSend(paneSend('device'))
  // A watcher firing is not a request anyone is waiting on, so a closed window
  // is an ordinary outcome rather than a failure — the opposite of `paneSend`,
  // where a dropped send would leave the caller waiting out a timeout.
  setArtifactNotify((sessionId, path) => {
    if (!mainWindow || mainWindow.isDestroyed()) return
    mainWindow.webContents.send(IPC.BROWSER_FILE_CHANGED, { sessionId, path })
  })
  // A companion outlives the app otherwise: it holds a unix socket and a booted
  // simulator, and nothing reaps it once Vorn is gone.
  installCompanionQuitHook()
  registerIpcHandlers()

  // Window control IPC handlers (Electron-only)
  ipcMain.on(IPC.WINDOW_MINIMIZE, () => mainWindow?.minimize())
  ipcMain.on(IPC.WINDOW_MAXIMIZE, () => {
    if (mainWindow?.isMaximized()) mainWindow.unmaximize()
    else mainWindow?.maximize()
  })
  ipcMain.on(IPC.WINDOW_CLOSE, () => mainWindow?.close())
  ipcMain.handle(IPC.WINDOW_IS_MAXIMIZED, () => mainWindow?.isMaximized() ?? false)

  createMenu(toggleWidget)
  createWindow()

  if (!mainWindow) {
    log.error('[main] Failed to create main window')
    app.quit()
    return
  }

  // Wire server notifications → renderer/widget
  wireServerNotifications(bridge)

  // Decrypt connector credentials via safeStorage and push plaintext into
  // the server's in-memory store. Runs once on boot, re-syncs on every
  // config change so newly added connections are picked up without restart.
  installConnectorCredentialsSync(bridge)

  // Load config for widget + update channel
  let updateChannel: 'stable' | 'beta' = 'stable'
  let updateAutoDownload = true
  try {
    const config = await bridge.request<{
      defaults: {
        widgetEnabled?: boolean
        updateChannel?: 'stable' | 'beta'
        updateAutoDownload?: boolean
      }
    }>(IPC.CONFIG_LOAD)
    widgetEnabled = config.defaults.widgetEnabled !== false
    updateChannel = config.defaults.updateChannel ?? 'stable'
    updateAutoDownload = config.defaults.updateAutoDownload !== false
  } catch {
    // Config not available yet, use defaults
  }

  updateManager.init(mainWindow, updateChannel, updateAutoDownload)

  // Auto show/hide widget based on main window focus
  mainWindow.on('blur', () => {
    setTimeout(() => {
      if (widgetWindow?.isFocused()) return
      showWidget()
    }, 100)
  })

  mainWindow.on('focus', () => {
    hideWidget()
  })

  // Update IPC handlers
  ipcMain.on(IPC.UPDATE_INSTALL, () => {
    updateManager.installUpdate()
  })

  ipcMain.on(IPC.UPDATE_SET_CHANNEL, (_e, channel: 'stable' | 'beta') => {
    updateManager.setChannel(channel)
    updateManager.checkForUpdates()
  })

  ipcMain.on(IPC.UPDATE_CHECK, () => {
    updateManager.checkForUpdates()
  })

  ipcMain.on(IPC.UPDATE_SET_AUTO_DOWNLOAD, (_e, enabled: boolean) => {
    updateManager.setAutoDownload(enabled)
  })

  ipcMain.on(IPC.UPDATE_DOWNLOAD, () => {
    updateManager.downloadUpdate()
  })

  // Synchronous: the Updates panel reads this on mount, before any event has
  // had a chance to arrive.
  ipcMain.on(IPC.UPDATE_GET_STATUS, (event) => {
    event.returnValue = updateManager.getStatus()
  })

  ipcMain.on(IPC.WIDGET_HIDE, () => {
    hideWidget()
  })

  ipcMain.on(IPC.WIDGET_FOCUS_TERMINAL, (_, terminalId: string) => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.show()
      mainWindow.focus()
      mainWindow.webContents.send('widget:select-terminal', terminalId)
    }
  })

  ipcMain.on('widget:show-app', () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.show()
      mainWindow.focus()
    }
  })

  ipcMain.on(IPC.WIDGET_RENDERER_STATUS, () => {
    // Request widget update from server
    bridge.request('widget:requestUpdate').catch(() => {})
  })

  ipcMain.on(IPC.WIDGET_SET_ENABLED, (_, enabled: boolean) => {
    widgetEnabled = enabled
    if (!enabled) hideWidget()
  })

  // Widget view mode resize
  const VIEW_SIZES = { full: { w: 280, h: 400 }, compact: { w: 140, h: 36 } }
  ipcMain.on('widget:set-view-mode', (_, mode: 'full' | 'compact') => {
    if (!widgetWindow || widgetWindow.isDestroyed()) return
    const display = screen.getPrimaryDisplay()
    const { width: screenW, height: screenH } = display.workAreaSize
    const [oldX, oldY] = widgetWindow.getPosition()
    const [oldW, oldH] = widgetWindow.getSize()
    const { w, h } = VIEW_SIZES[mode]
    let newX = oldX + (oldW - w)
    let newY = oldY + (oldH - h)
    newX = Math.max(0, Math.min(newX, screenW - w))
    newY = Math.max(0, Math.min(newY, screenH - h))
    widgetWindow.setSize(w, h)
    widgetWindow.setPosition(newX, newY)
  })

  // Permission response from widget or main window → forward to server
  ipcMain.on(
    IPC.WIDGET_PERMISSION_RESPONSE,
    (
      _,
      {
        requestId,
        allow,
        updatedPermissions,
        updatedInput
      }: {
        requestId: string
        allow: boolean
        updatedPermissions?: unknown[]
        updatedInput?: unknown
      }
    ) => {
      bridge
        .request('permission:resolve', { requestId, allow, updatedPermissions, updatedInput })
        .catch((err) => log.error('[main] permission resolve failed:', err))
      updatePermissionShortcuts()
    }
  )

  app.on('activate', () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.show()
      mainWindow.focus()
    } else if (BrowserWindow.getAllWindows().length === 0) {
      createWindow()
    }
  })
})

// The updater takes the process down by a path that closes windows before
// before-quit, so the close handler below has to know a quit is underway
// before it decides to cancel one. updateManager owns the subscription; this
// only records the fact.
updateManager.onQuitForUpdate(() => {
  isQuitting = true
})

app.on('before-quit', async () => {
  isQuitting = true
  globalShortcut.unregisterAll()
  updateManager.stop()
  // Killing the companions leaves the simulators Vorn booted running: nothing
  // releases a claim on the way out, and `bootedByVorn` is only honoured by
  // release. Detached, so this outlives the process rather than delaying it.
  deviceRegistry.shutdownOwnedDevices()
  await stopServer()
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit()
  }
})

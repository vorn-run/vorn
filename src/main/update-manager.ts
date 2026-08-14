import { autoUpdater, UpdateInfo, ProgressInfo } from 'electron-updater'
import { BrowserWindow, app } from 'electron'
import { IPC, UpdateStatus } from '../shared/types'
import log from './logger'

export type UpdateChannel = 'stable' | 'beta'

class UpdateManager {
  private mainWindow: BrowserWindow | null = null
  private checkInterval: ReturnType<typeof setInterval> | null = null
  /**
   * The last thing we told the renderer. Held so a Settings panel opened long
   * after the event can render the real state instead of a blank one — the
   * events fire once and are gone.
   *
   * Starts as `unsupported` because that is true until init() proves otherwise:
   * an unpackaged build never runs the updater at all.
   */
  private status: UpdateStatus = { kind: 'unsupported' }
  private lastCheckedAt: number | null = null

  init(mainWindow: BrowserWindow, channel: UpdateChannel = 'stable', autoDownload = true): void {
    if (!app.isPackaged) return

    this.mainWindow = mainWindow
    autoUpdater.autoDownload = autoDownload
    autoUpdater.autoInstallOnAppQuit = true
    this.setChannel(channel)
    this.setStatus({ kind: 'idle', lastCheckedAt: null })

    autoUpdater.on('checking-for-update', () => {
      this.setStatus({ kind: 'checking' })
    })

    autoUpdater.on('update-available', (info: UpdateInfo) => {
      this.lastCheckedAt = Date.now()
      // With autoDownload on, this is a step on the way to 'downloading' and
      // barely shows. With it off, this is where we stop until asked.
      this.setStatus({ kind: 'available', version: info.version })
    })

    autoUpdater.on('update-not-available', () => {
      this.lastCheckedAt = Date.now()
      this.setStatus({ kind: 'idle', lastCheckedAt: this.lastCheckedAt })
    })

    autoUpdater.on('download-progress', (progress: ProgressInfo) => {
      const current = this.status
      this.setStatus({
        kind: 'downloading',
        // The progress event carries no version, so carry it across from
        // whichever state we came from rather than losing it mid-download.
        version:
          current.kind === 'downloading' || current.kind === 'available' ? current.version : '',
        percent: Math.round(progress.percent)
      })
    })

    autoUpdater.on('update-downloaded', (info: UpdateInfo) => {
      this.lastCheckedAt = Date.now()
      this.setStatus({ kind: 'ready', version: info.version })
      this.mainWindow?.webContents.send(IPC.UPDATE_DOWNLOADED, {
        version: info.version
      })
    })

    autoUpdater.on('error', (err) => {
      log.error('[updater] Error:', err.message)
      // Previously logged and dropped, which made a failed update completely
      // invisible: the app just never offered one.
      this.setStatus({ kind: 'error', message: err.message })
    })

    this.checkForUpdates()
    this.checkInterval = setInterval(() => this.checkForUpdates(), 4 * 60 * 60 * 1000)
  }

  /**
   * Set the update channel. 'beta' receives both beta and stable releases.
   * 'stable' (default) receives only stable releases.
   */
  setChannel(channel: UpdateChannel): void {
    autoUpdater.channel = channel === 'beta' ? 'beta' : 'latest'
    autoUpdater.allowPrerelease = channel === 'beta'
    log.info(`[updater] channel set to "${channel}" (allowPrerelease=${channel === 'beta'})`)
  }

  setAutoDownload(enabled: boolean): void {
    autoUpdater.autoDownload = enabled
  }

  checkForUpdates(): void {
    autoUpdater.checkForUpdates().catch((err) => {
      log.error('[updater] Check failed:', err.message)
      this.setStatus({ kind: 'error', message: err.message })
    })
  }

  /** Start the transfer the user deferred by turning auto-download off. */
  downloadUpdate(): void {
    autoUpdater.downloadUpdate().catch((err) => {
      log.error('[updater] Download failed:', err.message)
      this.setStatus({ kind: 'error', message: err.message })
    })
  }

  getStatus(): UpdateStatus {
    return this.status
  }

  installUpdate(): void {
    autoUpdater.quitAndInstall(false, true)
  }

  private setStatus(status: UpdateStatus): void {
    this.status = status
    this.mainWindow?.webContents.send(IPC.UPDATE_STATUS, status)
  }

  stop(): void {
    if (this.checkInterval) {
      clearInterval(this.checkInterval)
      this.checkInterval = null
    }
  }
}

export const updateManager = new UpdateManager()

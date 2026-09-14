import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { autoUpdater, UpdateInfo, ProgressInfo } from 'electron-updater'
import { BrowserWindow, app, dialog, autoUpdater as nativeAutoUpdater } from 'electron'
import { IPC, UpdateStatus } from '../shared/types'
import { isNewerVersion } from '../shared/version-order'
import log from './logger'

export type UpdateChannel = 'stable' | 'beta'

/** The version last handed to the installer, so the next launch can tell whether it landed. */
const attemptFile = (): string => path.join(app.getPath('userData'), 'update-attempt.txt')
const shipItLog = (): string =>
  path.join(os.homedir(), 'Library/Caches/com.vorn.app.ShipIt/ShipIt_stderr.log')
/** How much of the installer's log a failed-update notice quotes, and how far back it reads. */
const SHIPIT_LINES = 6
const SHIPIT_TAIL_BYTES = 16 * 1024

/** The last lines the macOS installer wrote, or nothing elsewhere or when there is no log to read. */
function shipItTail(): string {
  if (process.platform !== 'darwin') return ''
  let fd: number | undefined
  try {
    fd = fs.openSync(shipItLog(), 'r')
    const { size } = fs.fstatSync(fd)
    const length = Math.min(size, SHIPIT_TAIL_BYTES)
    const buffer = Buffer.alloc(length)
    fs.readSync(fd, buffer, 0, length, size - length)
    const lines = buffer
      .toString('utf-8')
      .split('\n')
      .filter((line) => line.trim() !== '')
    return lines.slice(-SHIPIT_LINES).join('\n')
  } catch {
    return ''
  } finally {
    if (fd !== undefined) fs.closeSync(fd)
  }
}

export class UpdateManager {
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
  /** On macOS Squirrel stages the download itself, after electron-updater reports it finished. */
  private stagedBySquirrel = false
  /** The downloaded update, and whether the installer holds it complete so a restart installs it. */
  private update: { version: string; staged: boolean } | null = null

  /**
   * Claim the quit before the windows are asked to close.
   *
   * quitAndInstall() emits before-quit *after* closing every window, the
   * reverse of a normal quit, so a close handler that cancels the close until
   * it knows a quit is underway would cancel the updater's own quit — which is
   * exactly what left the app hidden instead of restarting. The signal lives on
   * Electron's native autoUpdater rather than on `app`, and it belongs here,
   * beside the quitAndInstall it guards, rather than in the window code.
   */
  onQuitForUpdate(handler: () => void): void {
    nativeAutoUpdater.on('before-quit-for-update', handler)
  }

  init(mainWindow: BrowserWindow, channel: UpdateChannel = 'stable', autoDownload = true): void {
    if (!app.isPackaged) return

    this.mainWindow = mainWindow
    this.stagedBySquirrel = process.platform === 'darwin'
    this.reportFailedAttempt(mainWindow)
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
      const percent = Math.round(progress.percent)
      // The event fires per received chunk — tens of times a second — and the
      // rounded percent is identical across most of them. Dropping the repeats
      // here keeps a multi-hundred-MB download to ~100 IPC messages instead of
      // tens of thousands, each of which would re-render every subscriber.
      if (current.kind === 'downloading' && current.percent === percent) return
      this.setStatus({
        kind: 'downloading',
        // The progress event carries no version, so carry it across from
        // whichever state we came from rather than losing it mid-download.
        version:
          current.kind === 'downloading' || current.kind === 'available' ? current.version : '',
        percent
      })
    })

    autoUpdater.on('update-downloaded', (info: UpdateInfo) => {
      this.lastCheckedAt = Date.now()
      this.update = { version: info.version, staged: !this.stagedBySquirrel }
      // On macOS Squirrel still has to unpack it; a restart before then relaunches the old build.
      this.setStatus(
        this.update.staged
          ? { kind: 'ready', version: info.version }
          : { kind: 'downloading', version: info.version, percent: 100 }
      )
    })

    if (this.stagedBySquirrel) {
      nativeAutoUpdater.on('update-downloaded', () => {
        if (!this.update) return
        this.update = { ...this.update, staged: true }
        this.setStatus({ kind: 'ready', version: this.update.version })
      })
    }

    autoUpdater.on('error', (err) => {
      log.error('[updater] Error:', err.message)
      // A Squirrel-staged update still installs, so the button that installs it stays.
      if (this.holdsSquirrelStage()) return
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
    // A check rebuilds Squirrel's feed and deletes what it staged, so none runs until it installs.
    if (this.holdsSquirrelStage()) return
    autoUpdater.checkForUpdates().catch((err) => {
      log.error('[updater] Check failed:', err.message)
      this.setStatus({ kind: 'error', message: err.message })
    })
  }

  /** Start the transfer the user deferred by turning auto-download off. */
  downloadUpdate(): void {
    if (this.holdsSquirrelStage()) return
    autoUpdater.downloadUpdate().catch((err) => {
      log.error('[updater] Download failed:', err.message)
      this.setStatus({ kind: 'error', message: err.message })
    })
  }

  getStatus(): UpdateStatus {
    return this.status
  }

  /** Install the staged update, running `release` just before; nothing happens, nothing released, until it is staged. */
  async installUpdate(release: () => Promise<void>): Promise<void> {
    if (!this.update?.staged) {
      log.warn('[updater] asked to install before the update was staged; staying open')
      return
    }
    await release()
    this.recordAttempt(this.update.version)
    autoUpdater.quitAndInstall(false, true)
  }

  /** Only Squirrel throws a staged update away on a new check; Windows and Linux keep checking as before. */
  private holdsSquirrelStage(): boolean {
    return this.stagedBySquirrel && this.update?.staged === true
  }

  private recordAttempt(target: string): void {
    try {
      fs.writeFileSync(attemptFile(), target)
    } catch (err) {
      log.warn(`[updater] could not note the update attempt: ${String(err)}`)
    }
  }

  /** Say so once when the update last handed to the installer did not land. */
  private reportFailedAttempt(window: BrowserWindow): void {
    let target: string
    try {
      target = fs.readFileSync(attemptFile(), 'utf-8').trim()
    } catch {
      return
    }
    fs.rmSync(attemptFile(), { force: true })
    const current = app.getVersion()
    if (!target || !isNewerVersion(target, current)) return
    log.warn(`[updater] the update to ${target} did not install; still on ${current}`)
    const tail = shipItTail()
    void dialog.showMessageBox(window, {
      type: 'warning',
      message: `The update to ${target} did not install`,
      detail: `Vorn is still on ${current} and will offer the update again.${
        tail ? `\n\nThe installer reported:\n${tail}` : ''
      }`
    })
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

import type { EventEmitter } from 'node:events'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BrowserWindow } from 'electron'

const dirs = vi.hoisted(() => ({ userData: '' }))

vi.mock('electron-updater', async () => {
  const { EventEmitter } = await import('node:events')
  const autoUpdater = Object.assign(new EventEmitter(), {
    autoDownload: true,
    autoInstallOnAppQuit: false,
    channel: '',
    allowPrerelease: false,
    checkForUpdates: vi.fn(async () => null),
    downloadUpdate: vi.fn(async () => []),
    quitAndInstall: vi.fn()
  })
  return { autoUpdater }
})

vi.mock('electron', async () => {
  const { EventEmitter } = await import('node:events')
  return {
    app: {
      isPackaged: true,
      getVersion: vi.fn(() => '0.7.1-beta.3'),
      getPath: vi.fn(() => dirs.userData)
    },
    dialog: { showMessageBox: vi.fn(async () => ({ response: 0 })) },
    autoUpdater: new EventEmitter(),
    BrowserWindow: class {}
  }
})

vi.mock('../src/main/logger', () => ({ default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() } }))

const { autoUpdater } = await import('electron-updater')
const electron = await import('electron')
const { UpdateManager } = await import('../src/main/update-manager')

const updater = autoUpdater as unknown as EventEmitter & {
  checkForUpdates: ReturnType<typeof vi.fn>
  downloadUpdate: ReturnType<typeof vi.fn>
  quitAndInstall: ReturnType<typeof vi.fn>
}
const squirrel = electron.autoUpdater as unknown as EventEmitter
const showMessageBox = electron.dialog.showMessageBox as unknown as ReturnType<typeof vi.fn>
const getVersion = electron.app.getVersion as unknown as ReturnType<typeof vi.fn>
const win = { webContents: { send: vi.fn() } } as unknown as BrowserWindow
const realPlatform = process.platform

const onPlatform = (platform: NodeJS.Platform): void => {
  Object.defineProperty(process, 'platform', { value: platform, configurable: true })
}
const attemptFile = (): string => path.join(dirs.userData, 'update-attempt.json')

let manager: InstanceType<typeof UpdateManager>

beforeEach(() => {
  dirs.userData = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-updater-'))
  updater.removeAllListeners()
  squirrel.removeAllListeners()
  vi.clearAllMocks()
  getVersion.mockReturnValue('0.7.1-beta.3')
  manager = new UpdateManager()
})

afterEach(() => {
  manager.stop()
  onPlatform(realPlatform)
  fs.rmSync(dirs.userData, { recursive: true, force: true })
})

const release = () => vi.fn(async () => {})

describe('a macOS update', () => {
  beforeEach(() => onPlatform('darwin'))

  it('offers the restart only once Squirrel has staged the download', async () => {
    manager.init(win, 'beta')
    updater.emit('update-downloaded', { version: '0.7.1-beta.4' })
    expect(manager.getStatus()).toEqual({
      kind: 'downloading',
      version: '0.7.1-beta.4',
      percent: 100
    })

    const early = release()
    await expect(manager.installUpdate(early)).resolves.toBe(false)
    expect(early).not.toHaveBeenCalled()
    expect(updater.quitAndInstall).not.toHaveBeenCalled()

    squirrel.emit('update-downloaded')
    expect(manager.getStatus()).toEqual({ kind: 'ready', version: '0.7.1-beta.4' })
  })

  it('stops checking once staged, so nothing throws the staged update away', () => {
    manager.init(win, 'beta')
    updater.emit('update-downloaded', { version: '0.7.1-beta.4' })
    squirrel.emit('update-downloaded')
    updater.checkForUpdates.mockClear()

    manager.checkForUpdates()
    manager.downloadUpdate()
    expect(updater.checkForUpdates).not.toHaveBeenCalled()
    expect(updater.downloadUpdate).not.toHaveBeenCalled()

    updater.emit('error', new Error('feed rebuilt'))
    expect(manager.getStatus()).toEqual({ kind: 'ready', version: '0.7.1-beta.4' })
  })

  it('releases the server before installing, and notes the attempt', async () => {
    manager.init(win, 'beta')
    updater.emit('update-downloaded', { version: '0.7.1-beta.4' })
    squirrel.emit('update-downloaded')

    const order: string[] = []
    const releasing = vi.fn(async () => {
      order.push('release')
    })
    updater.quitAndInstall.mockImplementation(() => order.push('install'))
    await expect(manager.installUpdate(releasing)).resolves.toBe(true)
    expect(order).toEqual(['release', 'install'])
    expect(JSON.parse(fs.readFileSync(attemptFile(), 'utf-8'))).toEqual({
      from: '0.7.1-beta.3',
      target: '0.7.1-beta.4'
    })
  })
})

describe('a Windows update', () => {
  beforeEach(() => onPlatform('win32'))

  it('offers the restart as soon as the download is done, as before', async () => {
    manager.init(win, 'beta')
    updater.emit('update-downloaded', { version: '0.7.1-beta.4' })
    expect(manager.getStatus()).toEqual({ kind: 'ready', version: '0.7.1-beta.4' })
    await expect(manager.installUpdate(release())).resolves.toBe(true)
    expect(updater.quitAndInstall).toHaveBeenCalledWith(false, true)
  })

  it('keeps checking, downloading and reporting errors after a download, as before', () => {
    manager.init(win, 'beta')
    updater.emit('update-downloaded', { version: '0.7.1-beta.4' })
    updater.checkForUpdates.mockClear()

    manager.checkForUpdates()
    manager.downloadUpdate()
    expect(updater.checkForUpdates).toHaveBeenCalledTimes(1)
    expect(updater.downloadUpdate).toHaveBeenCalledTimes(1)

    updater.emit('error', new Error('feed unreachable'))
    expect(manager.getStatus()).toEqual({ kind: 'error', message: 'feed unreachable' })
  })
})

describe('the launch after an update', () => {
  beforeEach(() => onPlatform('darwin'))

  it('says once that the update did not install, and forgets the attempt', () => {
    fs.writeFileSync(
      attemptFile(),
      JSON.stringify({ from: '0.7.1-beta.3', target: '0.7.1-beta.4' })
    )
    manager.init(win, 'beta')
    expect(showMessageBox).toHaveBeenCalledWith(
      win,
      expect.objectContaining({
        message: 'The update to 0.7.1-beta.4 did not install',
        detail: expect.stringContaining('Vorn is still on 0.7.1-beta.3')
      })
    )
    expect(fs.existsSync(attemptFile())).toBe(false)
  })

  it('stays quiet when the update landed', () => {
    getVersion.mockReturnValue('0.7.1-beta.4')
    fs.writeFileSync(
      attemptFile(),
      JSON.stringify({ from: '0.7.1-beta.3', target: '0.7.1-beta.4' })
    )
    manager.init(win, 'beta')
    expect(showMessageBox).not.toHaveBeenCalled()
    expect(fs.existsSync(attemptFile())).toBe(false)
  })
})

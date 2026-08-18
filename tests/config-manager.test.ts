import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const mockInitDb = vi.fn()
const mockCloseDb = vi.fn()
const mockLoadConfig = vi.fn()
const mockSaveConfig = vi.fn()
const mockDataDir = '/tmp/vorn-test-data-dir'

vi.mock('../packages/server/src/database', () => ({
  initDatabase: (dataDir?: string) => mockInitDb(dataDir),
  closeDatabase: () => mockCloseDb(),
  // The watcher watches wherever the database actually landed, so it asks the
  // database rather than rebuilding the default path itself.
  getDataDir: () => mockDataDir,
  loadConfig: () => mockLoadConfig(),
  saveConfig: (...args: unknown[]) => mockSaveConfig(...args)
}))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', () => ({
  default: {
    watch: vi.fn(() => ({ close: vi.fn() })),
    existsSync: vi.fn(() => true),
    mkdirSync: vi.fn()
  }
}))

import fs from 'node:fs'

beforeEach(() => {
  vi.clearAllMocks()
  vi.resetModules()
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

async function getConfigManager() {
  const mod = await import('../packages/server/src/config-manager')
  return mod.configManager
}

describe('configManager', () => {
  it('init calls initDatabase', async () => {
    const cm = await getConfigManager()
    cm.init()
    expect(mockInitDb).toHaveBeenCalled()
  })

  it('loadConfig returns DB result on success', async () => {
    const config = { version: 1, defaults: { shell: '/bin/zsh' }, projects: [] }
    mockLoadConfig.mockReturnValueOnce(config)
    const cm = await getConfigManager()
    expect(cm.loadConfig()).toEqual(config)
  })

  it('loadConfig returns defaults on error', async () => {
    mockLoadConfig.mockImplementationOnce(() => {
      throw new Error('db error')
    })
    const cm = await getConfigManager()
    const result = cm.loadConfig()
    expect(result.version).toBe(1)
    expect(result.defaults).toBeDefined()
  })

  it('saveConfig delegates to DB', async () => {
    const config = { version: 1, defaults: { shell: '/bin/zsh' } }
    const cm = await getConfigManager()
    cm.saveConfig(config as never)
    expect(mockSaveConfig).toHaveBeenCalledWith(config)
  })

  it('saveConfig throws on DB error', async () => {
    mockSaveConfig.mockImplementationOnce(() => {
      throw new Error('write failed')
    })
    const cm = await getConfigManager()
    expect(() => cm.saveConfig({} as never)).toThrow('write failed')
  })

  it('onConfigChanged + notifyChanged calls callbacks with fresh config', async () => {
    const config = { version: 1, defaults: { shell: '/bin/zsh' }, projects: [] }
    mockLoadConfig.mockReturnValue(config)

    const cm = await getConfigManager()
    const callback = vi.fn()
    cm.onConfigChanged(callback)
    cm.notifyChanged()
    expect(callback).toHaveBeenCalledWith(config)
  })

  it('watchDb calls fs.watch on the DB directory', async () => {
    const cm = await getConfigManager()
    cm.watchDb()
    // Specifically the directory the database reported, not a second copy of
    // the default: a server on its own --data-dir must watch its own files.
    expect(fs.watch).toHaveBeenCalledWith(mockDataDir, expect.any(Function))
  })

  it('watchDb triggers notifyChanged for .db-signal files', async () => {
    let watchCallback: (event: string, filename: string) => void = () => {}
    vi.mocked(fs.watch).mockImplementation((_path: unknown, cb: unknown) => {
      watchCallback = cb as typeof watchCallback
      return { close: vi.fn() } as unknown as fs.FSWatcher
    })

    const config = { version: 1, defaults: { shell: '/bin/zsh' }, projects: [] }
    mockLoadConfig.mockReturnValue(config)

    const cm = await getConfigManager()
    const callback = vi.fn()
    cm.onConfigChanged(callback)
    cm.watchDb()

    watchCallback('change', '.db-signal')
    await vi.advanceTimersByTimeAsync(300)

    expect(callback).toHaveBeenCalledWith(config)
  })

  it('watchDb triggers notifyChanged for .db-wal files', async () => {
    let watchCallback: (event: string, filename: string) => void = () => {}
    vi.mocked(fs.watch).mockImplementation((_path: unknown, cb: unknown) => {
      watchCallback = cb as typeof watchCallback
      return { close: vi.fn() } as unknown as fs.FSWatcher
    })

    const config = { version: 1, defaults: { shell: '/bin/zsh' }, projects: [] }
    mockLoadConfig.mockReturnValue(config)

    const cm = await getConfigManager()
    const callback = vi.fn()
    cm.onConfigChanged(callback)
    cm.watchDb()

    watchCallback('change', 'vorn.db-wal')
    await vi.advanceTimersByTimeAsync(300)

    expect(callback).toHaveBeenCalledWith(config)
  })

  it('watchDb triggers notifyChanged for .db files', async () => {
    let watchCallback: (event: string, filename: string) => void = () => {}
    vi.mocked(fs.watch).mockImplementation((_path: unknown, cb: unknown) => {
      watchCallback = cb as typeof watchCallback
      return { close: vi.fn() } as unknown as fs.FSWatcher
    })

    const config = { version: 1, defaults: { shell: '/bin/zsh' }, projects: [] }
    mockLoadConfig.mockReturnValue(config)

    const cm = await getConfigManager()
    const callback = vi.fn()
    cm.onConfigChanged(callback)
    cm.watchDb()

    watchCallback('change', 'vorn.db')
    await vi.advanceTimersByTimeAsync(300)

    expect(callback).toHaveBeenCalledWith(config)
  })

  it('watchDb ignores unrelated files', async () => {
    let watchCallback: (event: string, filename: string) => void = () => {}
    vi.mocked(fs.watch).mockImplementation((_path: unknown, cb: unknown) => {
      watchCallback = cb as typeof watchCallback
      return { close: vi.fn() } as unknown as fs.FSWatcher
    })

    mockLoadConfig.mockReturnValue({ version: 1 })

    const cm = await getConfigManager()
    const callback = vi.fn()
    cm.onConfigChanged(callback)
    cm.watchDb()

    watchCallback('change', 'something.json')
    watchCallback('change', 'notes.txt')
    watchCallback('change', '')
    await vi.advanceTimersByTimeAsync(500)

    expect(callback).not.toHaveBeenCalled()
  })

  it('watchDb debounces rapid events into a single notification', async () => {
    let watchCallback: (event: string, filename: string) => void = () => {}
    vi.mocked(fs.watch).mockImplementation((_path: unknown, cb: unknown) => {
      watchCallback = cb as typeof watchCallback
      return { close: vi.fn() } as unknown as fs.FSWatcher
    })

    const config = { version: 1, defaults: { shell: '/bin/zsh' }, projects: [] }
    mockLoadConfig.mockReturnValue(config)

    const cm = await getConfigManager()
    const callback = vi.fn()
    cm.onConfigChanged(callback)
    cm.watchDb()

    // Fire 5 rapid events — should coalesce into 1 notification
    watchCallback('change', '.db-signal')
    watchCallback('change', 'vorn.db-wal')
    watchCallback('change', 'vorn.db')
    watchCallback('change', '.db-signal')
    watchCallback('change', '.db-signal')
    await vi.advanceTimersByTimeAsync(300)

    expect(callback).toHaveBeenCalledTimes(1)
  })

  it('close calls closeDatabase and stops watcher', async () => {
    const cm = await getConfigManager()
    cm.close()
    expect(mockCloseDb).toHaveBeenCalled()
  })
})

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import type { PathSource } from '../src/main/binary-path'

/** A bridge that answers `env:path` with whatever the test wants. */
const sourceOf = (answer: () => Promise<unknown>): PathSource =>
  ({ request: answer }) as unknown as PathSource

/**
 * Finding a binary from a process launched without a shell.
 *
 * The case that matters is the bug this module was written for: an app
 * started from Finder has `/usr/bin:/bin:/usr/sbin:/sbin` and nothing else,
 * while `idb_companion` sits in Homebrew's directory.
 */
vi.mock('../src/main/logger', () => ({
  default: { debug: () => {}, info: () => {}, warn: () => {} }
}))

const importModule = async () => {
  // The server's PATH is cached in module state, so each test starts fresh.
  vi.resetModules()
  return import('../src/main/binary-path')
}

/** Places the given absolute paths on disk, and nothing else. */
function onDisk(...executables: string[]): void {
  vi.spyOn(fs, 'accessSync').mockImplementation((p) => {
    if (!executables.includes(String(p))) throw new Error('ENOENT')
  })
  vi.spyOn(fs, 'statSync').mockImplementation(
    (p) => ({ isFile: () => executables.includes(String(p)) }) as unknown as fs.Stats
  )
}

const FINDER_PATH = '/usr/bin:/bin:/usr/sbin:/sbin'
const origPath = process.env.PATH
const origOverride = process.env.VORN_IDB_COMPANION

beforeEach(() => {
  process.env.PATH = FINDER_PATH
  delete process.env.VORN_IDB_COMPANION
})

afterEach(() => {
  process.env.PATH = origPath
  if (origOverride === undefined) delete process.env.VORN_IDB_COMPANION
  else process.env.VORN_IDB_COMPANION = origOverride
  vi.restoreAllMocks()
})

describe('resolving a binary the app was not told about', () => {
  it('finds a Homebrew install that the Finder PATH hides', async () => {
    const mod = await importModule()
    onDisk('/opt/homebrew/bin/idb_companion')

    const found = mod.resolveBinary('idb_companion')
    expect(found.path).toBe('/opt/homebrew/bin/idb_companion')
    // Every system directory was tried first, and is worth naming in an error.
    expect(found.searched.slice(0, 4)).toEqual(['/usr/bin', '/bin', '/usr/sbin', '/sbin'])
    expect(found.searched).toContain('/opt/homebrew/bin')
  })

  it('reports where it looked when there is nothing to find', async () => {
    const mod = await importModule()
    onDisk()

    const missing = mod.resolveBinary('idb_companion')
    expect(missing.path).toBeNull()
    expect(missing.searched).toContain('/opt/homebrew/bin')
    expect(missing.searched).toContain('/usr/local/bin')
    expect(missing.overrideMiss).toBeUndefined()
  })

  it('prefers the PATH the server read from a login shell', async () => {
    const mod = await importModule()
    onDisk('/Users/someone/.local/bin/idb_companion', '/opt/homebrew/bin/idb_companion')
    mod.setPathSource(
      sourceOf(async () => ({ path: '/Users/someone/.local/bin:/usr/bin', resolved: true }))
    )
    await mod.primeHostPath()

    expect(mod.resolveBinary('idb_companion').path).toBe('/Users/someone/.local/bin/idb_companion')
  })

  it('takes an absolute override, and says so when it names nothing runnable', async () => {
    const mod = await importModule()
    onDisk('/somewhere/else/idb_companion')

    process.env.VORN_IDB_COMPANION = '/somewhere/else/idb_companion'
    expect(mod.resolveBinary('idb_companion', 'VORN_IDB_COMPANION').path).toBe(
      '/somewhere/else/idb_companion'
    )

    // A wrong override ends the search: the person who set it is owed that answer.
    process.env.VORN_IDB_COMPANION = '/typo/idb_companion'
    const bad = mod.resolveBinary('idb_companion', 'VORN_IDB_COMPANION')
    expect(bad).toMatchObject({ path: null, overrideMiss: '/typo/idb_companion', searched: [] })
  })
})

describe('asking the server for its PATH', () => {
  it('asks again while the answer is provisional, and stops once it is settled', async () => {
    const mod = await importModule()
    onDisk()
    const request = vi
      .fn()
      .mockResolvedValueOnce({ path: '/provisional/bin', resolved: false })
      .mockResolvedValueOnce({ path: '/opt/homebrew/bin', resolved: true })
    mod.setPathSource(sourceOf(request))

    await mod.primeHostPath()
    expect(mod.hostPath()).toBe('/provisional/bin')
    await mod.primeHostPath()
    expect(mod.hostPath()).toBe('/opt/homebrew/bin')
    await mod.primeHostPath()
    expect(request).toHaveBeenCalledTimes(2)
  })

  it('resolves without the server rather than failing when the bridge is down', async () => {
    const mod = await importModule()
    onDisk('/opt/homebrew/bin/idb_companion')
    mod.setPathSource(sourceOf(async () => Promise.reject(new Error('not connected'))))

    await expect(mod.primeHostPath()).resolves.toBeUndefined()
    expect(mod.hostPath()).toBeNull()
    const found = await mod.resolveBinaryWaiting('idb_companion', undefined, 10)
    expect(found.path).toBe('/opt/homebrew/bin/idb_companion')
  })

  it('waits for the server only when the ordinary places missed', async () => {
    const mod = await importModule()
    onDisk('/unusual/bin/idb_companion')
    const request = vi.fn().mockResolvedValue({ path: '/unusual/bin', resolved: true })
    mod.setPathSource(sourceOf(request))

    const found = await mod.resolveBinaryWaiting('idb_companion', undefined, 1_000)
    expect(found.path).toBe('/unusual/bin/idb_companion')

    // With it found where the search already looks, the server is never asked.
    const quiet = await importModule()
    onDisk('/opt/homebrew/bin/idb_companion')
    const second = vi.fn()
    quiet.setPathSource(sourceOf(second))
    await quiet.resolveBinaryWaiting('idb_companion', undefined, 1_000)
    expect(second).not.toHaveBeenCalled()
  })
})

describe('the PATH handed to a child', () => {
  it('carries the server PATH, the one this process has, and where Homebrew installs', async () => {
    const mod = await importModule()
    vi.spyOn(fs, 'existsSync').mockImplementation((p) => String(p) === '/opt/homebrew/bin')
    mod.setPathSource(sourceOf(async () => ({ path: '/shell/bin:/usr/bin', resolved: true })))
    await mod.primeHostPath()

    const dirs = mod.augmentedPath().split(':')
    expect(dirs[0]).toBe('/shell/bin')
    expect(dirs).toContain('/opt/homebrew/bin')
    expect(dirs).toContain('/sbin')
    // No repeats: /usr/bin is on both the server's PATH and this process's.
    expect(new Set(dirs).size).toBe(dirs.length)
  })
})

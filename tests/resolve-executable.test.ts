import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'

// We swap in a custom safe-env per test so we can drive the PATH search.
const safeEnvMock = vi.fn()
const shellAnswered = { value: true }
vi.mock('../packages/server/src/process-utils', () => ({
  getSafeEnv: () => safeEnvMock(),
  shellEnvResolved: () => shellAnswered.value
}))

const importResolver = async () => {
  // Reset module state so the per-name cache starts fresh each test.
  vi.resetModules()
  return import('../packages/server/src/resolve-executable')
}

const origPlatform = process.platform
function setPlatform(p: NodeJS.Platform): void {
  Object.defineProperty(process, 'platform', { value: p, configurable: true })
}

beforeEach(() => {
  shellAnswered.value = true
  safeEnvMock.mockReset()
})

afterEach(() => {
  setPlatform(origPlatform)
  vi.restoreAllMocks()
})

describe('resolveExecutable()', () => {
  it('returns null and re-probes on subsequent calls when not found', async () => {
    safeEnvMock.mockReturnValue({ PATH: '/nope' })
    const accessSpy = vi.spyOn(fs, 'accessSync').mockImplementation(() => {
      throw new Error('ENOENT')
    })
    const { resolveExecutable } = await importResolver()
    expect(resolveExecutable('nothing')).toBeNull()
    expect(resolveExecutable('nothing')).toBeNull()
    // Missing-lookups are NOT cached, so we re-probe each call.
    expect(accessSpy.mock.calls.length).toBeGreaterThanOrEqual(2)
  })

  it('caches successful lookups', async () => {
    safeEnvMock.mockReturnValue({ PATH: '/usr/bin' })
    const accessSpy = vi.spyOn(fs, 'accessSync').mockImplementation((p) => {
      if (String(p).endsWith('/usr/bin/thing')) return undefined
      throw new Error('ENOENT')
    })
    const { resolveExecutable } = await importResolver()
    const first = resolveExecutable('thing')
    const callsAfterFirst = accessSpy.mock.calls.length
    const second = resolveExecutable('thing')
    expect(second).toBe(first)
    // No additional probes on the second call.
    expect(accessSpy.mock.calls.length).toBe(callsAfterFirst)
  })

  it('does not keep a hit made before the login shell answered', async () => {
    safeEnvMock.mockReturnValue({ PATH: '/usr/bin' })
    const accessSpy = vi.spyOn(fs, 'accessSync').mockImplementation((p) => {
      if (String(p).endsWith('/usr/bin/git')) return undefined
      throw new Error('ENOENT')
    })
    shellAnswered.value = false
    const { resolveExecutable } = await importResolver()
    resolveExecutable('git')
    const probesBeforeAnswer = accessSpy.mock.calls.length
    resolveExecutable('git')
    // Still asked: the provisional PATH's git may not be the user's.
    expect(accessSpy.mock.calls.length).toBeGreaterThan(probesBeforeAnswer)
    shellAnswered.value = true
    resolveExecutable('git')
    const probesAfterAnswer = accessSpy.mock.calls.length
    expect(resolveExecutable('git')).toBe('/usr/bin/git')
    expect(accessSpy.mock.calls.length).toBe(probesAfterAnswer)
  })

  it('returns null when PATH is empty', async () => {
    safeEnvMock.mockReturnValue({})
    const { resolveExecutable } = await importResolver()
    expect(resolveExecutable('anything')).toBeNull()
  })

  it('tries .exe and .cmd suffixes on Windows', async () => {
    setPlatform('win32')
    safeEnvMock.mockReturnValue({ PATH: 'C:\\bin' })
    const checked: string[] = []
    vi.spyOn(fs, 'accessSync').mockImplementation((p) => {
      checked.push(String(p))
      if (String(p).endsWith('gh.cmd')) return undefined
      throw new Error('ENOENT')
    })
    const { resolveExecutable } = await importResolver()
    const resolved = resolveExecutable('gh')
    expect(resolved).toMatch(/gh\.cmd$/)
    expect(checked.some((c) => c.endsWith('gh.exe'))).toBe(true)
  })

  it('resetResolveCache(name) clears a specific entry', async () => {
    safeEnvMock.mockReturnValue({ PATH: '/usr/bin' })
    const accessSpy = vi.spyOn(fs, 'accessSync').mockImplementation((p) => {
      if (String(p).endsWith('/usr/bin/thing')) return undefined
      throw new Error('ENOENT')
    })
    const { resolveExecutable, resetResolveCache } = await importResolver()
    resolveExecutable('thing')
    const before = accessSpy.mock.calls.length
    resetResolveCache('thing')
    resolveExecutable('thing')
    expect(accessSpy.mock.calls.length).toBeGreaterThan(before)
  })

  it('resetResolveCache() with no arg clears all entries', async () => {
    safeEnvMock.mockReturnValue({ PATH: '/usr/bin' })
    const accessSpy = vi.spyOn(fs, 'accessSync').mockImplementation((p) => {
      if (String(p).endsWith('/usr/bin/a') || String(p).endsWith('/usr/bin/b')) return undefined
      throw new Error('ENOENT')
    })
    const { resolveExecutable, resetResolveCache } = await importResolver()
    resolveExecutable('a')
    resolveExecutable('b')
    const before = accessSpy.mock.calls.length
    resetResolveCache()
    resolveExecutable('a')
    resolveExecutable('b')
    expect(accessSpy.mock.calls.length).toBeGreaterThan(before)
  })
})

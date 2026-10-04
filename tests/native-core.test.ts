import fs from 'node:fs'
import path from 'node:path'
import { describe, it, expect, afterEach, vi } from 'vitest'
import {
  coreFor,
  coreStatus,
  forcedCoreMode,
  loadNativeCore,
  preloadFlaggedCore,
  resetCoreSelection,
  setExperimentalSource,
  nativeCoreCandidates,
  requestedCoreMode,
  selectCore,
  type NativeCore
} from '../packages/server/src/native-core'

const fakeCore: NativeCore = {
  info: () => ({ version: '0.0.0', ghostty: null }),
  hello: (name) => `hello ${name}`
}

describe('requestedCoreMode', () => {
  it('defaults to js when unset or empty', () => {
    expect(requestedCoreMode(undefined)).toBe('js')
    expect(requestedCoreMode('')).toBe('js')
    expect(requestedCoreMode('  ')).toBe('js')
  })

  it('accepts js and native in any case', () => {
    expect(requestedCoreMode('js')).toBe('js')
    expect(requestedCoreMode('Native ')).toBe('native')
  })

  it('rejects anything else', () => {
    expect(requestedCoreMode('rust')).toBeNull()
  })
})

describe('nativeCoreCandidates', () => {
  it('looks beside the packaged server, then in the checkout', () => {
    const dir = path.join('/app', 'resources', 'server')
    expect(nativeCoreCandidates(dir)).toEqual([
      path.join('/app', 'resources', 'core', 'vorn_core.node'),
      path.join('/app', 'core', 'vorn_core.node')
    ])
  })

  it('tries an explicit path first', () => {
    const [first] = nativeCoreCandidates('/x/server', '/opt/vorn_core.node')
    expect(first).toBe(path.resolve('/opt/vorn_core.node'))
  })
})

describe('loadNativeCore', () => {
  it('opens the first candidate that exists', () => {
    const opened: string[] = []
    const core = loadNativeCore(['/a.node', '/b.node', '/c.node'], {
      exists: (file) => file !== '/a.node',
      open: (file) => {
        opened.push(file)
        return fakeCore
      }
    })
    expect(opened).toEqual(['/b.node'])
    expect(core.hello('x')).toBe('hello x')
  })

  it('names every place it looked when nothing is there', () => {
    expect(() => loadNativeCore(['/a.node', '/b.node'], { exists: () => false })).toThrow(
      /vorn_core\.node not found.*\/a\.node, \/b\.node/
    )
  })

  it('refuses a binary that is not the core', () => {
    expect(() =>
      loadNativeCore(['/a.node'], { exists: () => true, open: () => ({ other: 1 }) })
    ).toThrow(/not a vorn core/)
  })

  it('surfaces a dlopen failure', () => {
    const file = path.join(fs.mkdtempSync(path.join(process.cwd(), '.tmp-core-')), 'bad.node')
    try {
      fs.writeFileSync(file, 'not a shared library')
      expect(() => loadNativeCore([file])).toThrow()
    } finally {
      fs.rmSync(path.dirname(file), { recursive: true, force: true })
    }
  })
})

describe('selectCore', () => {
  it('stays on js without loading anything when the flag is off', () => {
    let loaded = false
    const core = selectCore({
      env: {},
      load: () => {
        loaded = true
        return fakeCore
      }
    })
    expect(core).toEqual({ mode: 'js', native: null })
    expect(loaded).toBe(false)
  })

  it('reports an unrecognized value and stays on js', () => {
    expect(selectCore({ env: { VORN_CORE: 'rust' } })).toEqual({
      mode: 'js',
      native: null,
      fallback: 'unrecognized VORN_CORE=rust'
    })
  })

  it('loads the native core from the override and the server directory', () => {
    let seen: string[] = []
    const core = selectCore({
      env: { VORN_CORE: 'native', VORN_CORE_PATH: '/opt/vorn_core.node' },
      dir: '/app/resources/server',
      load: (candidates) => {
        seen = candidates
        return fakeCore
      }
    })
    expect(core.mode).toBe('native')
    expect(core.native?.hello('server')).toBe('hello server')
    expect(seen[0]).toBe(path.resolve('/opt/vorn_core.node'))
  })

  it('falls back to js with the reason when the binary will not load', () => {
    const core = selectCore({
      env: { VORN_CORE: 'native' },
      load: () => {
        throw new Error('vorn_core.node not found')
      }
    })
    expect(core).toEqual({ mode: 'js', native: null, fallback: 'vorn_core.node not found' })
  })

  it('falls back to js when a loaded binary throws from info()', () => {
    const core = selectCore({
      env: { VORN_CORE: 'native' },
      load: () => ({
        ...fakeCore,
        info: () => {
          throw new Error('stale binary')
        }
      })
    })
    expect(core).toEqual({ mode: 'js', native: null, fallback: 'stale binary' })
  })

  it('says why it fell back even when what was thrown is not an Error', () => {
    const core = selectCore({
      env: { VORN_CORE: 'native' },
      load: () => {
        throw 'dlopen said no'
      }
    })
    expect(core).toEqual({ mode: 'js', native: null, fallback: 'dlopen said no' })
  })

  it('keeps what the binary reported about itself', () => {
    const core = selectCore({ env: { VORN_CORE: 'native' }, load: () => fakeCore })
    expect(core.info).toEqual({ version: '0.0.0', ghostty: null })
  })

  it('looks in the default places when given nothing', () => {
    const core = selectCore({ env: { VORN_CORE: 'native', VORN_CORE_PATH: '/nonexistent/x.node' } })
    // Only meaningful as a smoke test of the defaults: either a built core is
    // in the checkout, or the fallback names what it looked for.
    if (core.native) expect(typeof core.native.info().version).toBe('string')
    else expect(core.fallback).toMatch(/nonexistent/)
  })
})

describe('experimental switches', () => {
  afterEach(() => {
    vi.unstubAllEnvs()
    setExperimentalSource(null)
    resetCoreSelection()
  })

  function countingLoader(): { loads: () => number; load: () => NativeCore } {
    let n = 0
    return {
      loads: () => n,
      load: () => {
        n++
        return fakeCore
      }
    }
  }

  it('reads VORN_CORE as forced only when it is set', () => {
    expect(forcedCoreMode(undefined)).toBeNull()
    expect(forcedCoreMode('  ')).toBeNull()
    expect(forcedCoreMode('Native')).toBe('native')
    expect(forcedCoreMode('js')).toBe('js')
    expect(forcedCoreMode('rust')).toBe('js')
  })

  it('stays on js, loading nothing, while every switch is off', () => {
    vi.stubEnv('VORN_CORE', '')
    const loader = countingLoader()
    resetCoreSelection(loader.load)
    setExperimentalSource(() => ({ nativeScreen: false }))
    expect(coreFor('screen')).toBeNull()
    expect(preloadFlaggedCore()).toBeNull()
    expect(loader.loads()).toBe(0)
  })

  it('loads the core once for a switch that is on, and follows the switch per call', () => {
    vi.stubEnv('VORN_CORE', '')
    const loader = countingLoader()
    resetCoreSelection(loader.load)
    let flags = { nativeScreen: true }
    setExperimentalSource(() => flags)
    expect(coreFor('screen')).toBe(fakeCore)
    expect(coreFor('screen')).toBe(fakeCore)
    flags = { nativeScreen: false }
    expect(coreFor('screen')).toBeNull()
    expect(loader.loads()).toBe(1)
  })

  it('lets VORN_CORE override the switches both ways', () => {
    const loader = countingLoader()
    resetCoreSelection(loader.load)
    setExperimentalSource(() => ({ nativeScreen: true }))
    vi.stubEnv('VORN_CORE', 'js')
    expect(coreFor('screen')).toBeNull()
    expect(coreStatus()).toEqual({
      loaded: null,
      version: null,
      error: null,
      forced: 'js',
      missing: []
    })
    vi.stubEnv('VORN_CORE', 'rust')
    expect(coreFor('screen')).toBeNull()
    expect(coreStatus()).toMatchObject({
      forced: 'js',
      error: 'VORN_CORE=rust is not recognized'
    })
    vi.stubEnv('VORN_CORE', 'native')
    setExperimentalSource(() => ({ nativeScreen: false }))
    expect(coreFor('screen')).toBe(fakeCore)
    expect(coreStatus()).toMatchObject({ loaded: true, version: '0.0.0', forced: 'native' })
  })

  it('treats a switch whose config cannot be read as off', () => {
    vi.stubEnv('VORN_CORE', '')
    resetCoreSelection(() => fakeCore)
    setExperimentalSource(() => {
      throw new Error('no database')
    })
    expect(coreFor('screen')).toBeNull()
  })

  it('reports a binary that will not load, and keeps the switch on js', () => {
    vi.stubEnv('VORN_CORE', '')
    resetCoreSelection(() => {
      throw new Error('vorn_core.node not found')
    })
    setExperimentalSource(() => ({ nativeScreen: true }))
    expect(coreFor('screen')).toBeNull()
    expect(coreStatus()).toEqual({
      loaded: false,
      version: null,
      error: 'vorn_core.node not found',
      forced: null,
      missing: []
    })
  })

  it('names the switches a binary was built without', () => {
    vi.stubEnv('VORN_CORE', '')
    resetCoreSelection(() => fakeCore)
    expect(coreStatus()).toMatchObject({ loaded: true, missing: ['nativeScreen', 'nativeGit'] })
    class Screen {}
    resetCoreSelection(() => ({ ...fakeCore, Screen }) as unknown as NativeCore)
    expect(coreStatus()).toMatchObject({ loaded: true, missing: ['nativeGit'] })
    const gitRun = async (): Promise<string> => ''
    resetCoreSelection(() => ({ ...fakeCore, Screen, gitRun }) as unknown as NativeCore)
    expect(coreStatus()).toMatchObject({ loaded: true, missing: [] })
  })
})

// Runs only where `yarn build:core` has produced the binary, as the core CI job does.
const builtCore = path.resolve(__dirname, '../packages/core/vorn_core.node')
describe.runIf(fs.existsSync(builtCore))('vorn_core.node', () => {
  it('answers a call from Node', () => {
    const core = loadNativeCore([builtCore])
    expect(core.hello('test')).toMatch(/^hello test from vorn-core \d+\.\d+\.\d+/)
    expect(core.info().version).toMatch(/^\d+\.\d+\.\d+/)
  })

  it('parses with libghostty-vt exactly when it reports being built with it', () => {
    const core = loadNativeCore([builtCore])
    if (core.parseTitle) {
      expect(core.parseTitle(Buffer.from('\x1b]2;vorn\x07'))).toBe('vorn')
      expect(core.info().ghostty).toBeTruthy()
    } else {
      expect(core.info().ghostty ?? null).toBeNull()
    }
  })
})

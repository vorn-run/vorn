import fs from 'node:fs'
import path from 'node:path'
import { describe, it, expect } from 'vitest'
import {
  loadNativeCore,
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

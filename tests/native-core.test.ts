import fs from 'node:fs'
import path from 'node:path'
import { describe, it, expect, afterEach } from 'vitest'
import {
  activeCore,
  coreStatus,
  loadNativeCore,
  nativeCore,
  resetCoreSelection,
  nativeCoreCandidates,
  selectCore,
  type NativeCore
} from '../packages/server/src/native-core'

const fakeCore: NativeCore = {
  info: () => ({ version: '0.0.0' }),
  hello: (name) => `hello ${name}`
}

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
  it('loads the core from the override and the server directory', () => {
    let seen: string[] = []
    const core = selectCore({
      env: { VORN_CORE_PATH: '/opt/vorn_core.node' },
      dir: '/app/resources/server',
      load: (candidates) => {
        seen = candidates
        return fakeCore
      }
    })
    expect(core.native?.hello('server')).toBe('hello server')
    expect(seen[0]).toBe(path.resolve('/opt/vorn_core.node'))
  })

  it('says why when the binary will not load', () => {
    const core = selectCore({
      env: {},
      load: () => {
        throw new Error('vorn_core.node not found')
      }
    })
    expect(core).toEqual({ native: null, error: 'vorn_core.node not found' })
  })

  it('counts a loaded binary that throws from info() as none', () => {
    const core = selectCore({
      env: {},
      load: () => ({
        ...fakeCore,
        info: () => {
          throw new Error('stale binary')
        }
      })
    })
    expect(core).toEqual({ native: null, error: 'stale binary' })
  })

  it('says why even when what was thrown is not an Error', () => {
    const core = selectCore({
      env: {},
      load: () => {
        throw 'dlopen said no'
      }
    })
    expect(core).toEqual({ native: null, error: 'dlopen said no' })
  })

  it('keeps what the binary reported about itself', () => {
    const core = selectCore({ env: {}, load: () => fakeCore })
    expect(core.info).toEqual({ version: '0.0.0' })
  })

  it('looks in the default places when given nothing', () => {
    const core = selectCore({ env: { VORN_CORE_PATH: '/nonexistent/x.node' } })
    // Only meaningful as a smoke test of the defaults: either a built core is
    // in the checkout, or the error names what it looked for.
    if (core.native) expect(typeof core.native.info().version).toBe('string')
    else expect(core.error).toMatch(/nonexistent/)
  })
})

describe('the active core', () => {
  afterEach(() => {
    resetCoreSelection()
  })

  it('is loaded once and kept', () => {
    let loads = 0
    resetCoreSelection(() => {
      loads++
      return fakeCore
    })
    expect(nativeCore()).toBe(fakeCore)
    expect(activeCore().native).toBe(fakeCore)
    expect(loads).toBe(1)
  })

  it('reports a binary that will not load', () => {
    resetCoreSelection(() => {
      throw new Error('vorn_core.node not found')
    })
    expect(nativeCore()).toBeNull()
    expect(coreStatus()).toEqual({
      loaded: false,
      version: null,
      error: 'vorn_core.node not found',
      missing: []
    })
  })

  it('names what a binary was built without', () => {
    resetCoreSelection(() => fakeCore)
    expect(coreStatus()).toEqual({
      loaded: true,
      version: '0.0.0',
      error: null,
      missing: ['git off the main thread', 'the native store']
    })
    class NativeStore {}
    const gitRun = async (): Promise<string> => ''
    resetCoreSelection(() => ({ ...fakeCore, gitRun, NativeStore }) as unknown as NativeCore)
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

  it('exports git and the store, and nothing for terminals, which run in vornd', () => {
    const core = loadNativeCore([builtCore]) as unknown as Record<string, unknown>
    expect(typeof core.gitRun).toBe('function')
    expect(typeof core.NativeStore).toBe('function')
    expect(core.TerminalPipeline).toBeUndefined()
    expect(core.Analyzer).toBeUndefined()
  })
})

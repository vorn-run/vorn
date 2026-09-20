// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { EventEmitter } from 'node:events'
import { Readable } from 'node:stream'

/**
 * What the companion is started as.
 *
 * The bug this covers: an app launched from Finder has only the system
 * directories on its PATH, so `spawn('idb_companion', …)` failed with ENOENT
 * and the person was told to install what they already had.
 */
const spawned: { cmd: string; args: string[]; env?: NodeJS.ProcessEnv }[] = []

function fakeChild(): EventEmitter & { stdout: Readable; stderr: Readable; kill: () => void } {
  const child = new EventEmitter() as EventEmitter & {
    stdout: Readable
    stderr: Readable
    kill: () => void
  }
  child.stdout = new Readable({ read() {} })
  child.stderr = new Readable({ read() {} })
  child.kill = () => {}
  return child
}

let nextChild = fakeChild()

vi.mock('node:child_process', () => ({
  spawn: (cmd: string, args: string[], opts: { env?: NodeJS.ProcessEnv }) => {
    spawned.push({ cmd, args, env: opts?.env })
    // Ready as soon as anyone listens, so startCompanion gets past the wait.
    queueMicrotask(() => nextChild.stdout.push(`{"grpc_path":"/tmp/x.sock"}\n`))
    return nextChild
  }
}))
vi.mock('electron', () => ({ app: { on: () => {}, getPath: () => '/tmp' } }))
vi.mock('@grpc/grpc-js', () => ({
  credentials: { createInsecure: () => ({}) },
  loadPackageDefinition: () => ({ idb: { CompanionService: class {} } })
}))
vi.mock('@grpc/proto-loader', () => ({ loadSync: () => ({}) }))
vi.mock('../src/main/logger', () => ({
  default: { debug: () => {}, info: () => {}, warn: () => {} }
}))

const resolution = {
  value: { path: '/opt/homebrew/bin/idb_companion', searched: ['/usr/bin'] } as {
    path: string | null
    searched: string[]
    overrideMiss?: string
  }
}
vi.mock('../src/main/binary-path', () => ({
  resolveBinaryWaiting: async () => resolution.value,
  augmentedPath: () => '/opt/homebrew/bin:/usr/bin'
}))

const importModule = async () => {
  vi.resetModules()
  return import('../src/main/device-companion')
}

const origResourcesPath = process.resourcesPath

beforeEach(() => {
  // Outside Electron there is no resourcesPath, and connect() builds the proto
  // path from it. The loader itself is mocked, so any string will do.
  Object.defineProperty(process, 'resourcesPath', { value: '/tmp', configurable: true })
  spawned.length = 0
  nextChild = fakeChild()
  resolution.value = { path: '/opt/homebrew/bin/idb_companion', searched: ['/usr/bin'] }
})

afterEach(() => {
  Object.defineProperty(process, 'resourcesPath', {
    value: origResourcesPath,
    configurable: true
  })
  vi.restoreAllMocks()
})

describe('starting the simulator companion', () => {
  it('runs the resolved binary, not a bare name, with a PATH it can work from', async () => {
    const mod = await importModule()
    await mod.startCompanion('ABCDEF01-0000-0000-0000-000000000000', () => {})

    expect(spawned).toHaveLength(1)
    expect(spawned[0].cmd).toBe('/opt/homebrew/bin/idb_companion')
    expect(spawned[0].args).toContain('--udid')
    expect(spawned[0].env?.PATH).toBe('/opt/homebrew/bin:/usr/bin')
  })

  it('says where it looked, and starts nothing, when the binary is absent', async () => {
    const mod = await importModule()
    resolution.value = { path: null, searched: ['/usr/bin', '/opt/homebrew/bin'] }

    await expect(
      mod.startCompanion('ABCDEF02-0000-0000-0000-000000000000', () => {})
    ).rejects.toThrow(/Looked in \/usr\/bin, \/opt\/homebrew\/bin/)
    expect(spawned).toHaveLength(0)
  })
})

describe('what the failure tells a person', () => {
  it('keeps the install line the picker shows', async () => {
    const mod = await importModule()
    const message = mod.companionMissingMessage({ searched: ['/usr/bin'] })
    expect(message).toContain('brew install facebook/fb/idb-companion')
    expect(message).toContain('idb_companion is not installed')
  })

  it('names the override when that is what is wrong, rather than blaming the install', async () => {
    const mod = await importModule()
    const message = mod.companionMissingMessage({ overrideMiss: '/typo/idb_companion' })
    expect(message).toContain('VORN_IDB_COMPANION points at /typo/idb_companion')
    expect(message).toContain('brew install facebook/fb/idb-companion')
    expect(message).not.toContain('idb_companion is not installed')
  })

  it('caps how many directories it lists, so a long PATH stays readable', async () => {
    const mod = await importModule()
    const dirs = Array.from({ length: 9 }, (_, i) => `/dir${i}`)
    const message = mod.companionMissingMessage({ searched: dirs })
    expect(message).toContain('and 3 more')
    expect(message).not.toContain('/dir6')
  })
})

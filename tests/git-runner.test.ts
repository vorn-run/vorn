import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const ssh = vi.hoisted(() => ({
  sync: vi.fn((): string => 'sync out'),
  async: vi.fn(async (): Promise<string> => 'async out')
}))

vi.mock('../packages/server/src/process-utils', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  sshExecSync: ssh.sync,
  sshExec: ssh.async
}))

import {
  DEFAULT_MAX_BUFFER,
  gitRunner,
  jsRunner,
  pinnedGitMode,
  resetGitRunner
} from '../packages/server/src/git-runner'
import {
  resetCoreSelection,
  setExperimentalSource,
  type NativeCore,
  type NativeGitRequest
} from '../packages/server/src/native-core'
import type { ExperimentalConfig, RemoteHost } from '../packages/shared/src/types'

const host = { id: 'h', hostname: 'box', user: 'dev', port: 22 } as RemoteHost

let flags: ExperimentalConfig = {}
const setFlags = (next: ExperimentalConfig): void => {
  flags = next
}

/** Loads `exports` as the binary would, or fails as a missing build does. */
function useBinary(exports: Partial<NativeCore> | null): void {
  resetCoreSelection(() => {
    if (!exports) throw new Error('vorn_core.node not found')
    return { info: () => ({ version: '0.0.0-test' }), hello: () => 'hi', ...exports } as NativeCore
  })
}

beforeEach(() => {
  delete process.env.VORN_CORE
  setExperimentalSource(() => flags)
  useBinary(null)
})

afterEach(() => {
  delete process.env.VORN_GIT
  delete process.env.VORN_CORE
  flags = {}
  setExperimentalSource(null)
  resetCoreSelection()
  resetGitRunner()
})

describe('which path git takes', () => {
  it('is the JS path until the switch is turned on', () => {
    useBinary({ gitRun: vi.fn() })
    expect(gitRunner()).toBe(jsRunner)
    setFlags({ nativeGit: true })
    expect(gitRunner().mode).toBe('native')
    // Another feature's switch leaves git where it was.
    setFlags({ nativeScreen: true })
    expect(gitRunner()).toBe(jsRunner)
  })

  it('lets VORN_GIT pin either path over the setting, for a benchmark or a test', () => {
    expect(pinnedGitMode({ VORN_GIT: ' Native ' })).toBe('native')
    expect(pinnedGitMode({ VORN_GIT: 'js' })).toBe('js')
    // Anything else is no pin at all, and the setting decides.
    expect(pinnedGitMode({ VORN_GIT: 'rust' })).toBeNull()
    expect(pinnedGitMode({})).toBeNull()

    useBinary({ gitRun: vi.fn() })
    setFlags({ nativeGit: true })
    process.env.VORN_GIT = 'js'
    expect(gitRunner()).toBe(jsRunner)
    setFlags({})
    process.env.VORN_GIT = 'native'
    expect(gitRunner().mode).toBe('native')
  })

  it('follows VORN_CORE when it forces every feature', () => {
    useBinary({ gitRun: vi.fn() })
    process.env.VORN_CORE = 'native'
    expect(gitRunner().mode).toBe('native')
    process.env.VORN_CORE = 'js'
    setFlags({ nativeGit: true })
    expect(gitRunner()).toBe(jsRunner)
  })

  it('stays on the JS path when the switch is on but the core cannot load', () => {
    setFlags({ nativeGit: true })
    expect(gitRunner()).toBe(jsRunner)
    process.env.VORN_GIT = 'native'
    expect(gitRunner()).toBe(jsRunner)
  })

  it('stays on the JS path when the loaded core predates gitRun', () => {
    useBinary({})
    setFlags({ nativeGit: true })
    expect(gitRunner()).toBe(jsRunner)
  })

  it('takes effect on the next call, without a restart', async () => {
    const gitRun = vi.fn(async () => 'from rust\n')
    useBinary({ gitRun })
    expect(gitRunner().mode).toBe('js')
    setFlags({ nativeGit: true })
    const runner = gitRunner()
    expect(runner.mode).toBe('native')
    expect(gitRunner()).toBe(runner)
    await expect(runner.local(['status'], '/repo', { timeout: 5000 })).resolves.toBe('from rust\n')
    setFlags({ nativeGit: false })
    expect(gitRunner()).toBe(jsRunner)
  })
})

describe('the native path', () => {
  it('hands the core what execFileSync would have been given', async () => {
    let seen: NativeGitRequest | undefined
    useBinary({
      gitRun: async (request: NativeGitRequest) => {
        seen = request
        return ''
      }
    })
    process.env.VORN_GIT = 'native'
    await gitRunner().local(['diff', '-U3'], '/repo', { timeout: 15000, maxBuffer: 1000 })
    expect(seen).toMatchObject({
      args: ['diff', '-U3'],
      cwd: '/repo',
      timeoutMs: 15000,
      maxBuffer: 1000
    })
    expect(seen?.bin).toMatch(/git/)
    expect(typeof seen?.env).toBe('object')
  })

  it("defaults the output limit to execFileSync's own", async () => {
    let limit: number | undefined
    useBinary({
      gitRun: async (request: NativeGitRequest) => {
        limit = request.maxBuffer
        return ''
      }
    })
    process.env.VORN_GIT = 'native'
    await gitRunner().local(['status'], '/repo', { timeout: 5000 })
    expect(limit).toBe(DEFAULT_MAX_BUFFER)
  })

  it('passes a git failure through as the rejection', async () => {
    useBinary({
      gitRun: async () => {
        throw new Error('Command failed: git checkout nope\nerror: pathspec')
      }
    })
    process.env.VORN_GIT = 'native'
    await expect(
      gitRunner().local(['checkout', 'nope'], '/repo', { timeout: 5000 })
    ).rejects.toThrow(/^Command failed: git checkout nope/)
  })
})

describe('a remote host', () => {
  it('blocks on the JS path, as sshExecSync always did', async () => {
    await expect(jsRunner.remote(host, 'git status', { timeout: 1000 })).resolves.toBe('sync out')
    expect(ssh.sync).toHaveBeenCalledWith(host, 'git status', { timeout: 1000 })
    ssh.sync.mockImplementationOnce(() => {
      throw new Error('ssh: connect to host box port 22: Connection refused')
    })
    await expect(jsRunner.remote(host, 'git status', { timeout: 1000 })).rejects.toThrow(
      /Connection refused/
    )
  })

  it('goes over async ssh on the native path, since it is a wait rather than work', async () => {
    useBinary({ gitRun: vi.fn() })
    process.env.VORN_GIT = 'native'
    await expect(gitRunner().remote(host, 'git status', { timeout: 1000 })).resolves.toBe(
      'async out'
    )
    expect(ssh.async).toHaveBeenCalledWith(host, 'git status', { timeout: 1000 })
  })
})

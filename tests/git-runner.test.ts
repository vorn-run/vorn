import { afterEach, describe, expect, it, vi } from 'vitest'

const binary = vi.hoisted(() => ({
  loaded: { native: null, error: 'vorn_core.node not found' } as {
    native: unknown
    error?: string
  }
}))

vi.mock('../packages/server/src/native-core', () => ({
  nativeBinary: () => binary.loaded
}))

import {
  DEFAULT_MAX_BUFFER,
  gitRunner,
  jsRunner,
  requestedGitMode,
  resetGitRunner
} from '../packages/server/src/git-runner'
import { setExperimentalFlags } from '../packages/server/src/experimental'
import type { NativeGitRequest } from '../packages/server/src/native-core'

afterEach(() => {
  delete process.env.VORN_GIT
  setExperimentalFlags({})
  resetGitRunner()
  binary.loaded = { native: null, error: 'vorn_core.node not found' }
})

describe('which path git takes', () => {
  it('is the JS path until the switch is turned on', () => {
    expect(requestedGitMode({})).toBe('js')
    setExperimentalFlags({ nativeGit: true })
    expect(requestedGitMode({})).toBe('native')
    setExperimentalFlags({ nativeGit: false })
    expect(requestedGitMode({})).toBe('js')
  })

  it('lets VORN_GIT pin either path over the setting, for a benchmark or a test', () => {
    setExperimentalFlags({ nativeGit: true })
    expect(requestedGitMode({ VORN_GIT: 'js' })).toBe('js')
    setExperimentalFlags({})
    expect(requestedGitMode({ VORN_GIT: ' Native ' })).toBe('native')
    // Anything else is no pin at all, and the setting decides.
    expect(requestedGitMode({ VORN_GIT: 'rust' })).toBe('js')
  })

  it('stays on the JS path when the switch is on but the core cannot load', () => {
    setExperimentalFlags({ nativeGit: true })
    expect(gitRunner()).toBe(jsRunner)
  })

  it('stays on the JS path when the loaded core predates gitRun', () => {
    binary.loaded = { native: { info: () => ({ version: '0.7.4' }) } }
    setExperimentalFlags({ nativeGit: true })
    expect(gitRunner()).toBe(jsRunner)
  })

  it('takes effect on the next call, without a restart', async () => {
    const gitRun = vi.fn(async () => 'from rust\n')
    binary.loaded = { native: { gitRun } }
    expect(gitRunner().mode).toBe('js')
    setExperimentalFlags({ nativeGit: true })
    const runner = gitRunner()
    expect(runner.mode).toBe('native')
    await expect(runner.local(['status'], '/repo', { timeout: 5000 })).resolves.toBe('from rust\n')
    setExperimentalFlags({ nativeGit: false })
    expect(gitRunner()).toBe(jsRunner)
  })
})

describe('the native path', () => {
  it('hands the core what execFileSync would have been given', async () => {
    let seen: NativeGitRequest | undefined
    binary.loaded = {
      native: {
        gitRun: async (request: NativeGitRequest) => {
          seen = request
          return ''
        }
      }
    }
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
    binary.loaded = {
      native: {
        gitRun: async (request: NativeGitRequest) => {
          limit = request.maxBuffer
          return ''
        }
      }
    }
    process.env.VORN_GIT = 'native'
    await gitRunner().local(['status'], '/repo', { timeout: 5000 })
    expect(limit).toBe(DEFAULT_MAX_BUFFER)
  })

  it('passes a git failure through as the rejection', async () => {
    binary.loaded = {
      native: {
        gitRun: async () => {
          throw new Error('Command failed: git checkout nope\nerror: pathspec')
        }
      }
    }
    process.env.VORN_GIT = 'native'
    await expect(
      gitRunner().local(['checkout', 'nope'], '/repo', { timeout: 5000 })
    ).rejects.toThrow(/^Command failed: git checkout nope/)
  })
})

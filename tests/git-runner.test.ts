import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const ssh = vi.hoisted(() => ({
  async: vi.fn(async (): Promise<string> => 'async out')
}))

vi.mock('../packages/server/src/process-utils', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  sshExec: ssh.async
}))

import {
  DEFAULT_MAX_BUFFER,
  gitRunner,
  processRunner,
  resetGitRunner
} from '../packages/server/src/git-runner'
import {
  resetCoreSelection,
  type NativeCore,
  type NativeGitRequest
} from '../packages/server/src/native-core'
import type { RemoteHost } from '../packages/shared/src/types'

const host = { id: 'h', hostname: 'box', user: 'dev', port: 22 } as RemoteHost

/** Loads `exports` as the binary would, or fails as a missing build does. */
function useBinary(exports: Partial<NativeCore> | null): void {
  resetCoreSelection(() => {
    if (!exports) throw new Error('vorn_core.node not found')
    return { info: () => ({ version: '0.0.0-test' }), hello: () => 'hi', ...exports } as NativeCore
  })
}

beforeEach(() => {
  useBinary(null)
})

afterEach(() => {
  resetCoreSelection()
  resetGitRunner()
})

describe('which path git takes', () => {
  it('is the core whenever it loaded with gitRun', async () => {
    const gitRun = vi.fn(async () => 'from rust\n')
    useBinary({ gitRun })
    const runner = gitRunner()
    expect(runner.mode).toBe('native')
    // Built once per core, not per call.
    expect(gitRunner()).toBe(runner)
    await expect(runner.local(['status'], '/repo', { timeout: 5000 })).resolves.toBe('from rust\n')
  })

  it('is a child process when the core cannot load', () => {
    expect(gitRunner()).toBe(processRunner)
  })

  it('is a child process when the loaded core has no gitRun', () => {
    useBinary({})
    expect(gitRunner()).toBe(processRunner)
  })
})

describe('the native path', () => {
  it('hands the core the command, where, and its limits', async () => {
    let seen: NativeGitRequest | undefined
    useBinary({
      gitRun: async (request: NativeGitRequest) => {
        seen = request
        return ''
      }
    })
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

  it('defaults the output limit to 1 MiB', async () => {
    let limit: number | undefined
    useBinary({
      gitRun: async (request: NativeGitRequest) => {
        limit = request.maxBuffer
        return ''
      }
    })
    await gitRunner().local(['status'], '/repo', { timeout: 5000 })
    expect(limit).toBe(DEFAULT_MAX_BUFFER)
  })

  it('passes a git failure through as the rejection', async () => {
    useBinary({
      gitRun: async () => {
        throw new Error('Command failed: git checkout nope\nerror: pathspec')
      }
    })
    await expect(
      gitRunner().local(['checkout', 'nope'], '/repo', { timeout: 5000 })
    ).rejects.toThrow(/^Command failed: git checkout nope/)
  })
})

describe('the child process path', () => {
  let dir: string

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-git-runner-'))
    execFileSync('git', ['init', '-q', '-b', 'main', dir])
  })

  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true })
  })

  it('answers with what git printed', async () => {
    await expect(
      processRunner.local(['rev-parse', '--is-inside-work-tree'], dir, { timeout: 5000 })
    ).resolves.toBe('true\n')
  })

  it("rejects with git's stderr on the error", async () => {
    const failure = await processRunner.local(['checkout', 'nope'], dir, { timeout: 5000 }).then(
      () => null,
      (err: unknown) => err as Error & { stderr?: string }
    )
    expect(failure).toBeInstanceOf(Error)
    expect(failure?.stderr).toMatch(/nope/)
  })
})

describe('a remote host', () => {
  it('goes over async ssh on either path, since it is a wait rather than work', async () => {
    await expect(processRunner.remote(host, 'git status', { timeout: 1000 })).resolves.toBe(
      'async out'
    )
    useBinary({ gitRun: vi.fn() })
    await expect(gitRunner().remote(host, 'git status', { timeout: 1000 })).resolves.toBe(
      'async out'
    )
    expect(ssh.async).toHaveBeenCalledWith(host, 'git status', { timeout: 1000 })
  })
})

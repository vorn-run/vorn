import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('node:child_process', async () => {
  const { EventEmitter } = await import('node:events')
  class FakeChild extends EventEmitter {
    pid = 4242
    stdin = { on: vi.fn(), end: vi.fn() }
    stdout = new EventEmitter()
    stderr = new EventEmitter()
    kill = vi.fn()
  }
  return {
    spawn: vi.fn(() => new FakeChild()),
    execFileSync: vi.fn(() => '/usr/bin/cmd')
  }
})

vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(() => 'main'),
  checkoutBranch: vi.fn(),
  createWorktree: vi.fn(),
  extractWorktreeName: vi.fn(),
  isGitRepo: vi.fn(() => false)
}))

import { spawn as spawnImport } from 'node:child_process'
import { headlessManager } from '../packages/server/src/headless-manager'

const spawnMock = spawnImport as unknown as ReturnType<typeof vi.fn>

describe('headlessManager.createHeadless', () => {
  beforeEach(() => {
    spawnMock.mockClear()
  })

  it('pins a fresh agentSessionId for claude and injects --session-id', () => {
    const session = headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true
    })

    expect(session.agentSessionId).toMatch(/^[0-9a-f-]{36}$/)

    const spawnCall = spawnMock.mock.calls[0]
    const args = spawnCall[1] as string[]
    const idx = args.indexOf('--session-id')
    expect(idx).toBeGreaterThanOrEqual(0)
    expect(args[idx + 1]).toBe(session.agentSessionId)

    headlessManager.killHeadless(session.id)
  })

  it('reuses resumeSessionId and uses --resume for claude', () => {
    const session = headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      resumeSessionId: 'existing-session',
      headless: true
    })

    expect(session.agentSessionId).toBe('existing-session')

    const args = spawnMock.mock.calls[0][1] as string[]
    expect(args).toContain('--resume')
    expect(args).toContain('existing-session')

    headlessManager.killHeadless(session.id)
  })

  it('does not populate agentSessionId for non-pinning agents', () => {
    const session = headlessManager.createHeadless({
      agentType: 'codex',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true
    })

    expect(session.agentSessionId).toBeUndefined()

    headlessManager.killHeadless(session.id)
  })

  it('propagates workflowId / workflowName onto the session', () => {
    const session = headlessManager.createHeadless({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      initialPrompt: 'go',
      headless: true,
      workflowId: 'wf-1',
      workflowName: 'wf'
    })

    expect(session.workflowId).toBe('wf-1')
    expect(session.workflowName).toBe('wf')

    headlessManager.killHeadless(session.id)
  })
})

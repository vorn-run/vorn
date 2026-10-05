import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

type ExecFileCb = (err: Error | null, stdout: string | Buffer, stderr: string | Buffer) => void

const foundCommands = new Set<string>()
const mockLoadConfig = vi.fn(() => ({ agentCommands: {} }))

vi.mock('node:child_process', () => ({
  execFile: (_cmd: string, args: string[], _opts: unknown, cb: ExecFileCb) => {
    const target = args?.[0] ?? _cmd
    if (foundCommands.has(target)) {
      cb(null, `/usr/local/bin/${target}`, '')
    } else {
      const err = new Error('not found') as NodeJS.ErrnoException
      err.code = 'ENOENT'
      cb(err, '', '')
    }
  },
  execFileSync: vi.fn(() => {
    throw new Error('mock shell env')
  })
}))

vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(),
  checkoutBranch: vi.fn(),
  createWorktree: vi.fn()
}))

vi.mock('../packages/server/src/config-manager', () => ({
  configManager: { loadConfig: mockLoadConfig }
}))

const originalEnv = process.env

describe('agent-detector', () => {
  beforeEach(async () => {
    vi.resetModules()
    foundCommands.clear()
    mockLoadConfig.mockReturnValue({ agentCommands: {} })
    process.env = {
      HOME: '/home/user',
      PATH: '/usr/local/bin:/usr/bin',
      SHELL: '/bin/zsh'
    }
  })

  afterEach(() => {
    process.env = originalEnv
  })

  it('returns all agents as not installed when all commands fail', async () => {
    const { detectInstalledAgents } = await import('../packages/server/src/agent-detector')
    const result = await detectInstalledAgents()
    expect(result.claude).toBe(false)
    expect(result.copilot).toBe(false)
    expect(result.codex).toBe(false)
    expect(result.opencode).toBe(false)
    expect(result.gemini).toBe(false)
  })

  it('detects installed agents when commands succeed', async () => {
    foundCommands.add('claude')
    foundCommands.add('copilot')
    const { detectInstalledAgents } = await import('../packages/server/src/agent-detector')
    const result = await detectInstalledAgents()
    expect(result.claude).toBe(true)
    expect(result.copilot).toBe(true)
    expect(result.codex).toBe(false)
  })

  it('caches results between calls', async () => {
    const { detectInstalledAgents } = await import('../packages/server/src/agent-detector')
    const result1 = await detectInstalledAgents()
    foundCommands.add('claude')
    const result2 = await detectInstalledAgents()
    expect(result1).toBe(result2)
    expect(result2.claude).toBe(false)
  })

  it('clears cache when clearAgentDetectionCache is called', async () => {
    const { detectInstalledAgents, clearAgentDetectionCache } =
      await import('../packages/server/src/agent-detector')
    await detectInstalledAgents()
    clearAgentDetectionCache()
    foundCommands.add('claude')
    const result = await detectInstalledAgents()
    expect(result.claude).toBe(true)
  })

  it('uses custom agent commands from config', async () => {
    mockLoadConfig.mockReturnValue({
      agentCommands: { claude: { command: 'my-claude', args: [] } }
    })
    foundCommands.add('my-claude')
    const { detectInstalledAgents } = await import('../packages/server/src/agent-detector')
    const result = await detectInstalledAgents()
    expect(result.claude).toBe(true)
  })

  it('strips sensitive env vars from detection subprocess', async () => {
    process.env = {
      HOME: '/home/user',
      PATH: '/usr/bin',
      SHELL: '/bin/zsh',
      GITHUB_TOKEN: 'ghp_secret',
      CLAUDECODE: 'nested'
    }
    const { detectInstalledAgents } = await import('../packages/server/src/agent-detector')
    await detectInstalledAgents()
    expect(true).toBe(true)
  })

  it('detects fallback commands when primary fails', async () => {
    mockLoadConfig.mockReturnValue({
      agentCommands: {
        claude: {
          command: 'claude-primary',
          args: [],
          fallbackCommand: 'claude-fallback',
          fallbackArgs: []
        }
      }
    })
    foundCommands.add('claude-fallback')
    const { detectInstalledAgents } = await import('../packages/server/src/agent-detector')
    const result = await detectInstalledAgents()
    expect(result.claude).toBe(true)
  })
})

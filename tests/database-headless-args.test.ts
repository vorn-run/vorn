import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, existsSync: vi.fn(() => true), mkdirSync: vi.fn() }
})

import { initTestDatabase, saveConfig, loadConfig } from '../packages/server/src/database'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import type { AppConfig } from '@vornrun/shared/types'

/** The commands a test just saved, refusing rather than assuming they came back. */
function commandsOf(config: AppConfig): NonNullable<AppConfig['agentCommands']> {
  const commands = config.agentCommands
  if (!commands) throw new Error('loadConfig returned a config with no agentCommands')
  return commands
}

let teardown: () => void

function makeConfig(overrides: Partial<AppConfig> = {}): AppConfig {
  return {
    version: 1,
    defaults: { theme: 'dark', shell: '/bin/zsh', fontSize: 13 },
    projects: [],
    agentCommands: { ...DEFAULT_AGENT_COMMANDS },
    workflows: [],
    tasks: [],
    ...overrides
  }
}

beforeEach(() => {
  teardown = initTestDatabase()
})

afterEach(() => {
  teardown()
})

describe('headlessArgs persistence (real SQLite)', () => {
  it('saves and loads headlessArgs for agent commands', () => {
    const config = makeConfig({
      agentCommands: {
        ...DEFAULT_AGENT_COMMANDS,
        claude: {
          ...DEFAULT_AGENT_COMMANDS.claude,
          headlessArgs: ['--dangerously-skip-permissions', '--verbose']
        }
      }
    })
    saveConfig(config)
    const loaded = loadConfig()
    expect(commandsOf(loaded).claude?.headlessArgs).toEqual([
      '--dangerously-skip-permissions',
      '--verbose'
    ])
  })

  it('loads headlessArgs as undefined when not set', () => {
    const config = makeConfig({
      agentCommands: {
        ...DEFAULT_AGENT_COMMANDS,
        opencode: { command: 'opencode', args: [] }
      }
    })
    saveConfig(config)
    const loaded = loadConfig()
    expect(commandsOf(loaded).opencode?.headlessArgs).toBeUndefined()
  })

  it('preserves headlessArgs across save/load round-trips', () => {
    const config = makeConfig()
    saveConfig(config)

    // Update with custom headlessArgs
    const loaded = loadConfig()
    commandsOf(loaded).gemini = {
      ...DEFAULT_AGENT_COMMANDS.gemini,
      headlessArgs: ['-y', '--no-confirm']
    }
    saveConfig(loaded)

    const reloaded = loadConfig()
    expect(commandsOf(reloaded).gemini?.headlessArgs).toEqual(['-y', '--no-confirm'])
    // Other agents should keep their defaults
    expect(commandsOf(reloaded).claude?.headlessArgs).toEqual(['--dangerously-skip-permissions'])
  })
})

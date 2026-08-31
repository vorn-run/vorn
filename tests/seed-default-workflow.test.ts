import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, existsSync: vi.fn(() => true), mkdirSync: vi.fn() }
})

import {
  initTestDatabase,
  seedSystemDefaults,
  loadConfig,
  saveConfig,
  dbDeleteWorkflow
} from '../packages/server/src/database'
import { DEFAULT_TASK_WORKFLOW_ID } from '../packages/server/src/default-workflows'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import type { AppConfig } from '@vornrun/shared/types'

let teardown: () => void

function baseConfig(overrides: Partial<AppConfig> = {}): AppConfig {
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

describe('seedSystemDefaults', () => {
  it('inserts the default task workflow on the first call', () => {
    seedSystemDefaults()

    const cfg = loadConfig()
    const seeded = cfg.workflows?.find((w) => w.id === DEFAULT_TASK_WORKFLOW_ID)
    expect(seeded).toBeDefined()
    expect(seeded?.name).toBe('Default Task Workflow')
    expect(seeded?.enabled).toBe(true)

    const triggerNode = seeded?.nodes.find((n) => n.type === 'trigger')
    expect(triggerNode).toBeDefined()
    const launchNode = seeded?.nodes.find((n) => n.type === 'launchAgent')
    expect(launchNode).toBeDefined()
  })

  it('sets the hasSeededDefaultTaskWorkflow flag after seeding', () => {
    seedSystemDefaults()

    const cfg = loadConfig()
    expect(cfg.defaults.hasSeededDefaultTaskWorkflow).toBe(true)
  })

  it('is idempotent — calling twice does not re-insert', () => {
    seedSystemDefaults()
    seedSystemDefaults()

    const cfg = loadConfig()
    const matching = (cfg.workflows ?? []).filter((w) => w.id === DEFAULT_TASK_WORKFLOW_ID)
    expect(matching).toHaveLength(1)
  })

  it('does NOT re-seed after the user deletes the default workflow', () => {
    seedSystemDefaults()
    dbDeleteWorkflow(DEFAULT_TASK_WORKFLOW_ID)

    // Now flag is already true — a subsequent seed call must be a no-op.
    seedSystemDefaults()

    const cfg = loadConfig()
    const seeded = cfg.workflows?.find((w) => w.id === DEFAULT_TASK_WORKFLOW_ID)
    expect(seeded).toBeUndefined()
    expect(cfg.defaults.hasSeededDefaultTaskWorkflow).toBe(true)
  })

  it('preserves the flag through a saveConfig/loadConfig round-trip', () => {
    seedSystemDefaults()
    const loaded = loadConfig()
    // Re-save to simulate the app writing config back to disk.
    saveConfig(loaded)

    const reloaded = loadConfig()
    expect(reloaded.defaults.hasSeededDefaultTaskWorkflow).toBe(true)
  })

  it('safety-net: if a workflow with the stable id already exists, does not insert a duplicate but still sets the flag', () => {
    // Pre-populate a workflow with the same id (simulating a partial/manual import).
    const pre = baseConfig({
      workflows: [
        {
          id: DEFAULT_TASK_WORKFLOW_ID,
          name: 'User Override',
          icon: 'Zap',
          iconColor: '#ff0000',
          nodes: [],
          edges: [],
          enabled: false,
          workspaceId: 'personal'
        }
      ]
    })
    saveConfig(pre)

    seedSystemDefaults()

    const cfg = loadConfig()
    const matches = (cfg.workflows ?? []).filter((w) => w.id === DEFAULT_TASK_WORKFLOW_ID)
    expect(matches).toHaveLength(1)
    // The user's override is preserved — not clobbered by the factory.
    expect(matches[0].name).toBe('User Override')
    expect(cfg.defaults.hasSeededDefaultTaskWorkflow).toBe(true)
  })

  it('seeds when the flag is present but set to false (never completed)', () => {
    // Explicitly set the flag to `false` — e.g. from a failed prior attempt or
    // a manual reset — and confirm seeding still runs.
    saveConfig(
      baseConfig({
        defaults: {
          theme: 'dark',
          shell: '/bin/zsh',
          fontSize: 13,
          hasSeededDefaultTaskWorkflow: false
        }
      })
    )

    seedSystemDefaults()

    const cfg = loadConfig()
    const seeded = cfg.workflows?.find((w) => w.id === DEFAULT_TASK_WORKFLOW_ID)
    expect(seeded).toBeDefined()
    expect(cfg.defaults.hasSeededDefaultTaskWorkflow).toBe(true)
  })
})

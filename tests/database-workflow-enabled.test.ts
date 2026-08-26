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
  dbListWorkflows,
  dbGetWorkflow,
  dbUpdateWorkflow,
  saveConfig
} from '../packages/server/src/database'
import type { AppConfig, WorkflowDefinition } from '@vornrun/shared/types'

/**
 * `dbUpdateWorkflow` writing what it says it writes.
 *
 * It is exported, it has an `enabled` branch, and until `workflow:setEnabled`
 * nothing in the repository called it — so every line of it was correct by
 * inspection and unproven by execution. That is precisely the shape of the
 * `project_name` bug in PR #488: a whitelist that read fine and had a column
 * missing from it, where the mocked socket suite went on passing because its
 * idea of what the function accepts is a hand-copy of somebody's belief.
 *
 * So this runs against the real table. If the branch is removed, this goes red
 * and `workflow-methods.test.ts` does not, which is the whole reason both exist.
 */

let teardown: () => void

beforeEach(() => {
  teardown = initTestDatabase()
})

afterEach(() => {
  teardown()
})

function seed(workflows: Partial<WorkflowDefinition>[]): void {
  saveConfig({
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark' },
    projects: [],
    tasks: [],
    // `icon` and `iconColor` are bound straight into the upsert with no fallback,
    // so leaving them undefined is not a default -- it is an unbindable value and
    // the insert throws. Every workflow in the real table has both.
    workflows: workflows.map((w) => ({
      icon: 'Zap',
      iconColor: '#3b82f6',
      nodes: [],
      edges: [],
      enabled: false,
      id: 'wf',
      name: 'A workflow',
      ...w
    })) as WorkflowDefinition[]
  } as AppConfig)
}

describe('switching a workflow on and off in the table', () => {
  it('turns one on, and the row says so', () => {
    seed([{ id: 'wf-a', name: 'clean branches', enabled: false }])

    dbUpdateWorkflow('wf-a', { enabled: true })

    expect(dbGetWorkflow('wf-a')?.enabled).toBe(true)
  })

  it('turns one off again', () => {
    // Both directions, because the column is an INTEGER and `false` is the value
    // a careless write drops: `enabled ? 1 : 0` is right, `enabled && 1` is not.
    seed([{ id: 'wf-a', name: 'clean branches', enabled: true }])

    dbUpdateWorkflow('wf-a', { enabled: false })

    expect(dbGetWorkflow('wf-a')?.enabled).toBe(false)
  })

  it('leaves the workflow otherwise as it was', () => {
    // An UPDATE that names one column should touch one column. Checked because
    // `dbUpdateWorkflow` builds its SET list by hand and a stray entry there
    // would blank a name or a set of nodes without anything failing.
    seed([
      {
        id: 'wf-a',
        name: 'Simple hello',
        icon: 'Cloud',
        iconColor: '#3b82f6',
        enabled: false,
        nodes: [{ id: 't', type: 'trigger', label: 'Manual Trigger', config: {} }]
      } as Partial<WorkflowDefinition>
    ])

    dbUpdateWorkflow('wf-a', { enabled: true })

    const after = dbGetWorkflow('wf-a')
    expect(after?.name).toBe('Simple hello')
    expect(after?.icon).toBe('Cloud')
    expect(after?.iconColor).toBe('#3b82f6')
    expect(after?.nodes).toHaveLength(1)
  })

  it('touches nobody else', () => {
    seed([
      { id: 'wf-a', name: 'Alpha', enabled: false },
      { id: 'wf-b', name: 'Beta', enabled: false }
    ])

    dbUpdateWorkflow('wf-a', { enabled: true })

    expect(dbGetWorkflow('wf-b')?.enabled).toBe(false)
  })

  it('reports how many rows it changed, which is how an unknown id is answered', () => {
    // `workflow:setEnabled` reads this count instead of fetching the workflow
    // first. That matters beyond tidiness: `dbGetWorkflow` parses `nodes` and
    // `edges`, so checking that way would let one malformed row throw a call
    // that only wanted to flip a boolean.
    seed([{ id: 'wf-a', name: 'Alpha', enabled: false }])

    expect(dbUpdateWorkflow('wf-a', { enabled: true })).toBe(1)
    expect(dbUpdateWorkflow('wf-nope', { enabled: true })).toBe(0)
    expect(dbListWorkflows()).toHaveLength(1)
  })

  it('counts the row it matched, not whether the value moved', () => {
    // The handler reads this count to answer an unknown id, so it depends on a
    // driver behaviour that is not obvious and is not universal: SQLite's
    // `changes` counts rows the UPDATE *matched*, so writing `true` over `true`
    // still reports one. MySQL's default `affected_rows` reports zero for the
    // same statement, which is where the opposite intuition comes from -- and
    // if this behaved that way, switching on a workflow that was already on
    // would come back `{ ok: false }` and the method would not be idempotent.
    seed([{ id: 'wf-a', name: 'Alpha', enabled: true }])

    expect(dbUpdateWorkflow('wf-a', { enabled: true })).toBe(1)
    expect(dbGetWorkflow('wf-a')?.enabled).toBe(true)
  })

  it('changes nothing, and says so, when handed no columns', () => {
    seed([{ id: 'wf-a', name: 'Alpha', enabled: false }])

    expect(dbUpdateWorkflow('wf-a', {})).toBe(0)
    expect(dbGetWorkflow('wf-a')?.name).toBe('Alpha')
  })
})

describe('listing the workflows', () => {
  it('hands back every one, including a workflow that has never run', () => {
    // The reason this method exists. `clean branches` has no run behind it, so
    // on the phone today it appears nowhere at all -- the Workflows tab lists
    // runs, and a workflow with none is invisible.
    seed([
      { id: 'wf-a', name: 'clean branches', enabled: false },
      { id: 'wf-b', name: 'Simple hello', enabled: true }
    ])

    const names = dbListWorkflows().map((w) => w.name)

    expect(names).toContain('clean branches')
    expect(names).toHaveLength(2)
  })

  it('carries what a row draws', () => {
    seed([
      {
        id: 'wf-a',
        name: 'Simple hello',
        icon: 'Cloud',
        iconColor: '#3b82f6',
        enabled: true,
        nodes: [
          {
            id: 't',
            type: 'trigger',
            label: 'Manual Trigger',
            config: { triggerType: 'manual', inputs: [{ key: 'pr_number', type: 'text' }] }
          }
        ]
      } as Partial<WorkflowDefinition>
    ])

    const [workflow] = dbListWorkflows()

    expect(workflow?.icon).toBe('Cloud')
    expect(workflow?.iconColor).toBe('#3b82f6')
    expect(workflow?.enabled).toBe(true)
    // The trigger travels inside `nodes`, which is why the list is not trimmed:
    // the row says "Manual · 1 input" and both halves come from here.
    const trigger = workflow?.nodes.find((n) => n.type === 'trigger')
    expect((trigger?.config as { triggerType?: string })?.triggerType).toBe('manual')
  })
})

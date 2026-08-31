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
  saveWorkflowRun,
  listWorkflowRuns,
  listAllWorkflowRuns,
  saveConfig
} from '../packages/server/src/database'
import type { AppConfig, WorkflowExecution } from '@vornrun/shared/types'
import { workflowRunId } from '@vornrun/shared/types'

let teardown: () => void

beforeEach(() => {
  teardown = initTestDatabase()
})

afterEach(() => {
  teardown()
})

describe('workflow run persistence', () => {
  it('round-trips agentType / projectName / projectPath on node states', () => {
    const exec: WorkflowExecution = {
      workflowId: 'wf-1',
      runId: 'wf-1:2026-04-20T10:00:00Z',
      startedAt: '2026-04-20T10:00:00Z',
      completedAt: '2026-04-20T10:00:05Z',
      status: 'success',
      nodeStates: [
        {
          nodeId: 'node-1',
          status: 'success',
          agentSessionId: 'agent-xyz',
          agentType: 'claude',
          projectName: 'proj',
          projectPath: '/abs/proj',
          approvedAt: '2026-04-20T10:00:04Z'
        }
      ]
    }

    saveWorkflowRun(exec)
    const runs = listWorkflowRuns('wf-1')
    expect(runs).toHaveLength(1)
    const state = runs[0].nodeStates[0]
    expect(state.agentType).toBe('claude')
    expect(state.projectName).toBe('proj')
    expect(state.projectPath).toBe('/abs/proj')
    expect(state.agentSessionId).toBe('agent-xyz')
    expect(state.approvedAt).toBe('2026-04-20T10:00:04Z')
  })

  it('round-trips step diagnostics, which outlive the window that made them', () => {
    // The timeline matters most for a run you come back to later, so it has to
    // survive the reload rather than living only in renderer memory.
    const timeline =
      '[+0.0s] Launching claude in /abs/proj\n' +
      '[+0.4s] Session sess-1 started (pid 4242): claude --dangerously-skip-permissions -p\n' +
      '[+3600.0s] Step timed out. The agent was started but never produced any output.'
    const exec: WorkflowExecution = {
      workflowId: 'wf-diag',
      runId: 'wf-diag:2026-04-20T10:00:00Z',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'error',
      nodeStates: [{ nodeId: 'node-1', status: 'error', diagnostics: timeline }]
    }

    saveWorkflowRun(exec)
    const runs = listWorkflowRuns('wf-diag')
    expect(runs[0].nodeStates[0].diagnostics).toBe(timeline)
  })

  it('round-trips the connector inbox row across approval-gate resumes', () => {
    const exec: WorkflowExecution = {
      workflowId: 'wf-connector',
      runId: 'wf-connector:2026-04-20T10:00:00Z',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'running',
      connectorInboxId: 73,
      connectorInboxLeaseToken: 'lease-73',
      connectorItem: {
        inboxId: 73,
        inboxLeaseToken: 'lease-73',
        connectionId: 'conn-1',
        connectorId: 'github',
        externalId: 'issue-73',
        title: 'Persist me',
        raw: { number: 73 }
      },
      nodeStates: [{ nodeId: 'approval', status: 'waiting' }]
    }

    saveWorkflowRun(exec)
    expect(listWorkflowRuns('wf-connector')[0]).toMatchObject({
      connectorInboxId: 73,
      connectorInboxLeaseToken: 'lease-73',
      connectorItem: {
        externalId: 'issue-73',
        title: 'Persist me',
        raw: { number: 73 }
      }
    })
  })

  it('omits fields that were not set', () => {
    const exec: WorkflowExecution = {
      workflowId: 'wf-2',
      runId: 'wf-2:2026-04-20T11:00:00Z',
      startedAt: '2026-04-20T11:00:00Z',
      status: 'success',
      nodeStates: [{ nodeId: 'node-1', status: 'success' }]
    }

    saveWorkflowRun(exec)
    const runs = listWorkflowRuns('wf-2')
    const state = runs[0].nodeStates[0]
    expect(state.agentType).toBeUndefined()
    expect(state.projectName).toBeUndefined()
    expect(state.projectPath).toBeUndefined()
  })

  it('never trims a running connector execution needed for restart recovery', () => {
    saveWorkflowRun({
      runId: 'active-connector-run',
      workflowId: 'wf-trim',
      startedAt: '2026-04-20T00:00:00Z',
      status: 'running',
      connectorInboxId: 500,
      connectorInboxLeaseToken: 'lease-500',
      nodeStates: [{ nodeId: 'agent', status: 'running' }]
    })
    for (let index = 0; index < 50; index++) {
      saveWorkflowRun({
        runId: `completed-${index}`,
        workflowId: 'wf-trim',
        startedAt: `2026-04-21T00:${String(index).padStart(2, '0')}:00Z`,
        completedAt: `2026-04-21T00:${String(index).padStart(2, '0')}:30Z`,
        status: 'success',
        nodeStates: []
      })
    }

    expect(
      listWorkflowRuns('wf-trim', 100).some((run) => run.runId === 'active-connector-run')
    ).toBe(true)
  })

  it('trims a finished connector run whose inbox row is already gone', () => {
    saveWorkflowRun({
      runId: 'orphan-connector-run',
      workflowId: 'wf-orphan',
      startedAt: '2026-04-20T00:00:00Z',
      completedAt: '2026-04-20T00:00:30Z',
      status: 'success',
      connectorInboxId: 900,
      nodeStates: []
    })
    for (let index = 0; index < 60; index++) {
      saveWorkflowRun({
        runId: `later-${index}`,
        workflowId: 'wf-orphan',
        startedAt: `2026-04-21T00:${String(index).padStart(2, '0')}:00Z`,
        completedAt: `2026-04-21T00:${String(index).padStart(2, '0')}:30Z`,
        status: 'success',
        nodeStates: []
      })
    }

    const runs = listWorkflowRuns('wf-orphan', 200)
    expect(runs.some((run) => run.runId === 'orphan-connector-run')).toBe(false)
    expect(runs.length).toBeLessThanOrEqual(51)
  })
})

function configWithWorkflows(
  workflows: { id: string; name: string; workspaceId?: string }[]
): AppConfig {
  return {
    version: 1,
    defaults: { shell: 'bash', fontSize: 14, theme: 'dark' },
    projects: [],
    workflows: workflows.map((w) => ({
      id: w.id,
      name: w.name,
      icon: 'Zap',
      iconColor: '#fff',
      nodes: [
        {
          id: 't',
          type: 'trigger',
          label: 'T',
          config: { triggerType: 'manual' },
          position: { x: 0, y: 0 }
        }
      ],
      edges: [],
      enabled: true,
      ...(w.workspaceId !== undefined && { workspaceId: w.workspaceId })
    }))
  }
}

describe('listAllWorkflowRuns', () => {
  it('returns runs across every workflow in started-desc order with workflow names attached', () => {
    saveConfig(
      configWithWorkflows([
        { id: 'wf-a', name: 'Alpha' },
        { id: 'wf-b', name: 'Beta' }
      ])
    )
    saveWorkflowRun({
      workflowId: 'wf-a',
      runId: 'wf-a:2026-04-20T10:00:00Z',
      startedAt: '2026-04-20T10:00:00Z',
      completedAt: '2026-04-20T10:00:05Z',
      status: 'success',
      nodeStates: [{ nodeId: 'n', status: 'success' }]
    })
    saveWorkflowRun({
      workflowId: 'wf-b',
      runId: 'wf-b:2026-04-20T10:01:00Z',
      startedAt: '2026-04-20T10:01:00Z',
      completedAt: '2026-04-20T10:01:09Z',
      status: 'error',
      nodeStates: [{ nodeId: 'n', status: 'error' }]
    })

    const runs = listAllWorkflowRuns()
    expect(runs.map((r) => r.workflowId)).toEqual(['wf-b', 'wf-a'])
    expect(runs.map((r) => r.workflowName)).toEqual(['Beta', 'Alpha'])
  })

  it('restricts to workflows in the given workspace', () => {
    saveConfig(
      configWithWorkflows([
        { id: 'wf-personal', name: 'P', workspaceId: 'personal' },
        { id: 'wf-team', name: 'T', workspaceId: 'team' }
      ])
    )
    saveWorkflowRun({
      workflowId: 'wf-personal',
      runId: 'wf-personal:2026-04-20T10:00:00Z',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'success',
      nodeStates: []
    })
    saveWorkflowRun({
      workflowId: 'wf-team',
      runId: 'wf-team:2026-04-20T10:00:30Z',
      startedAt: '2026-04-20T10:00:30Z',
      status: 'success',
      nodeStates: []
    })

    const personal = listAllWorkflowRuns('personal')
    expect(personal.map((r) => r.workflowId)).toEqual(['wf-personal'])

    const team = listAllWorkflowRuns('team')
    expect(team.map((r) => r.workflowId)).toEqual(['wf-team'])
  })

  it('honors the limit and clamps it to 500', () => {
    saveConfig(configWithWorkflows([{ id: 'wf-x', name: 'X' }]))
    for (let i = 0; i < 5; i++) {
      saveWorkflowRun({
        workflowId: 'wf-x',
        runId: `wf-x:2026-04-20T10:0${i}:00Z`,
        startedAt: `2026-04-20T10:0${i}:00Z`,
        status: 'success',
        nodeStates: []
      })
    }
    expect(listAllWorkflowRuns(undefined, 2)).toHaveLength(2)
    expect(listAllWorkflowRuns(undefined, 99999)).toHaveLength(5)
  })

  it('returns triggerTaskId only when present, and survives orphaned runs', () => {
    saveConfig(configWithWorkflows([{ id: 'wf-orphan', name: 'O' }]))
    saveWorkflowRun({
      workflowId: 'wf-orphan',
      runId: 'wf-orphan:2026-04-20T10:00:00Z',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'success',
      triggerTaskId: 'task-7',
      nodeStates: []
    })
    // Now wipe the workflow from config — the run row stays, name should be null.
    saveConfig({
      version: 1,
      defaults: { shell: 'bash', fontSize: 14, theme: 'dark' },
      projects: []
    })
    const runs = listAllWorkflowRuns()
    expect(runs).toHaveLength(1)
    expect(runs[0].triggerTaskId).toBe('task-7')
    expect(runs[0].workflowName).toBeUndefined()
  })

  it('excludes orphaned runs from workspace-filtered listings (no silent personal bucket)', () => {
    saveConfig(configWithWorkflows([{ id: 'wf-orphan', name: 'O', workspaceId: 'team' }]))
    saveWorkflowRun({
      workflowId: 'wf-orphan',
      runId: 'wf-orphan:2026-04-20T10:00:00Z',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'success',
      nodeStates: []
    })
    saveConfig({
      version: 1,
      defaults: { shell: 'bash', fontSize: 14, theme: 'dark' },
      projects: []
    })
    expect(listAllWorkflowRuns('personal')).toEqual([])
    expect(listAllWorkflowRuns('team')).toEqual([])
    expect(listAllWorkflowRuns()).toHaveLength(1)
  })
})

describe('workflow run inputs persistence', () => {
  it('round-trips the values a manual run was started with', () => {
    const exec: WorkflowExecution = {
      runId: 'run-inputs-1',
      workflowId: 'wf-inputs',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'success',
      nodeStates: [{ nodeId: 'n1', status: 'success' }],
      inputs: { issue: 'gh-42', count: 3, item: { number: 7 } }
    }
    saveWorkflowRun(exec)

    const [run] = listWorkflowRuns('wf-inputs')
    expect(run.inputs).toEqual({ issue: 'gh-42', count: 3, item: { number: 7 } })
  })

  it('ignores a non-object inputs blob rather than surfacing numeric keys', () => {
    saveWorkflowRun({
      runId: 'run-inputs-3',
      workflowId: 'wf-bad',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'success',
      nodeStates: [],
      // An array survives JSON.parse and is `typeof 'object'`, so without an
      // explicit check it would render as a row of numeric keys.
      inputs: ['a', 'b'] as unknown as Record<string, unknown>
    })

    const [run] = listWorkflowRuns('wf-bad')
    expect(run.inputs).toBeUndefined()
  })

  it('omits inputs entirely for a run that had none', () => {
    saveWorkflowRun({
      runId: 'run-inputs-2',
      workflowId: 'wf-none',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'success',
      nodeStates: []
    })

    const [run] = listWorkflowRuns('wf-none')
    expect(run.inputs).toBeUndefined()
  })
})

describe('step results survive a reload', () => {
  // These were held only in renderer memory: after a restart the typed verdict
  // was gone, {{steps.<slug>.<field>}} fell back to raw logs, and a run parked
  // on an approval gate — the one kind that outlives a restart — reported
  // "completed" for an agent that had said otherwise.
  const withResult = (over: Record<string, unknown> = {}): WorkflowExecution => ({
    workflowId: 'wf-results',
    runId: 'wf-results:2026-08-10T10:00:00Z',
    startedAt: '2026-08-10T10:00:00Z',
    status: 'success',
    nodeStates: [
      {
        nodeId: 'review-1',
        status: 'success',
        output: '2',
        structuredOutput: { approved: true, blocking: [], main_story: 'Meta open-weights' },
        iteration: 2,
        ...over
      }
    ]
  })

  it('round-trips output, structuredOutput and iteration', () => {
    saveWorkflowRun(withResult())
    const [run] = listWorkflowRuns('wf-results', 10)
    const node = run.nodeStates[0]
    expect(node.output).toBe('2')
    expect(node.structuredOutput).toEqual({
      approved: true,
      blocking: [],
      main_story: 'Meta open-weights'
    })
    expect(node.iteration).toBe(2)
  })

  it('keeps a nested typed value walkable, which is why it is not flattened', () => {
    saveWorkflowRun(
      withResult({ structuredOutput: { issue: { number: 42, url: 'https://example/42' } } })
    )
    const [run] = listWorkflowRuns('wf-results', 10)
    const nested = run.nodeStates[0].structuredOutput as { issue: { number: number } }
    expect(nested.issue.number).toBe(42)
  })

  it('omits the fields entirely when a step produced none', () => {
    saveWorkflowRun({
      workflowId: 'wf-bare',
      runId: 'wf-bare:2026-08-10T10:00:00Z',
      startedAt: '2026-08-10T10:00:00Z',
      status: 'success',
      nodeStates: [{ nodeId: 'plain', status: 'success' }]
    })
    const [run] = listWorkflowRuns('wf-bare', 10)
    expect(run.nodeStates[0]).not.toHaveProperty('structuredOutput')
    expect(run.nodeStates[0]).not.toHaveProperty('iteration')
  })

  it('degrades to no typed output rather than failing the whole run history', () => {
    // Stores an array, which serialises fine and parses fine but is not the
    // object shape a typed step output must be. The row must come back without
    // the field rather than with an undefined one, or `'structuredOutput' in
    // node` lies to every consumer downstream.
    saveWorkflowRun(withResult({ structuredOutput: [] as unknown as Record<string, unknown> }))
    const [run] = listWorkflowRuns('wf-results', 10)
    expect(run.nodeStates).toHaveLength(1)
    expect(run.nodeStates[0]).not.toHaveProperty('structuredOutput')
  })

  it('keeps the rest of the run when one typed output is unreadable', () => {
    saveWorkflowRun({
      workflowId: 'wf-mixed',
      runId: 'wf-mixed:2026-08-10T10:00:00Z',
      startedAt: '2026-08-10T10:00:00Z',
      status: 'success',
      nodeStates: [
        {
          nodeId: 'bad',
          status: 'success',
          structuredOutput: 'nope' as unknown as Record<string, unknown>
        },
        { nodeId: 'good', status: 'success', structuredOutput: { ok: true } }
      ]
    })
    const [run] = listWorkflowRuns('wf-mixed', 10)
    expect(run.nodeStates).toHaveLength(2)
    expect(run.nodeStates[0]).not.toHaveProperty('structuredOutput')
    expect(run.nodeStates[1].structuredOutput).toEqual({ ok: true })
  })
})

describe('identifying a run whose history predates the id', () => {
  /**
   * Every fixture in this file used to omit `runId` and lean on the fallback
   * without saying so. `runId` is required now and they all carry one, which is
   * correct -- and which quietly removed the only exercise this branch had. It
   * still runs in production every time a row written before the field is read
   * back, so it is tested here on purpose rather than by accident.
   */
  it('uses the id it was given', () => {
    expect(
      workflowRunId({ runId: 'chosen', workflowId: 'wf-1', startedAt: '2026-04-20T10:00:00Z' })
    ).toBe('chosen')
  })

  it('falls back to workflow and start time when there is none', () => {
    expect(workflowRunId({ workflowId: 'wf-1', startedAt: '2026-04-20T10:00:00Z' })).toBe(
      'wf-1:2026-04-20T10:00:00Z'
    )
  })

  it('treats an empty id as absent, because a blank is not an identity', () => {
    expect(
      workflowRunId({ runId: '', workflowId: 'wf-1', startedAt: '2026-04-20T10:00:00Z' })
    ).toBe('wf-1:2026-04-20T10:00:00Z')
  })
})

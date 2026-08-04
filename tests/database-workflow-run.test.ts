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
      startedAt: '2026-04-20T10:00:00Z',
      status: 'running',
      connectorInboxId: 73,
      nodeStates: [{ nodeId: 'approval', status: 'waiting' }]
    }

    saveWorkflowRun(exec)
    expect(listWorkflowRuns('wf-connector')[0].connectorInboxId).toBe(73)
  })

  it('omits fields that were not set', () => {
    const exec: WorkflowExecution = {
      workflowId: 'wf-2',
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
      startedAt: '2026-04-20T10:00:00Z',
      completedAt: '2026-04-20T10:00:05Z',
      status: 'success',
      nodeStates: [{ nodeId: 'n', status: 'success' }]
    })
    saveWorkflowRun({
      workflowId: 'wf-b',
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
      startedAt: '2026-04-20T10:00:00Z',
      status: 'success',
      nodeStates: []
    })
    saveWorkflowRun({
      workflowId: 'wf-team',
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

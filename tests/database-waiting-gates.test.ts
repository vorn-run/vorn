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
  listRunsWithWaitingGates
} from '../packages/server/src/database'
import type { WorkflowExecution } from '@vornrun/shared/types'

let teardown: () => void

beforeEach(() => {
  teardown = initTestDatabase()
})

afterEach(() => {
  teardown()
})

function run(
  id: string,
  status: 'running' | 'success' | 'error',
  nodeStatus: 'waiting' | 'success' | 'pending'
): WorkflowExecution {
  return {
    runId: `${id}:2026-04-20T10:00:0${id.slice(-1)}Z`,
    workflowId: id,
    startedAt: `2026-04-20T10:00:0${id.slice(-1)}Z`,
    status,
    nodeStates: [{ nodeId: 'gate', status: nodeStatus }]
  }
}

describe('listRunsWithWaitingGates', () => {
  it('returns an empty array when no runs have waiting nodes', () => {
    saveWorkflowRun(run('wf-1', 'success', 'success'))
    expect(listRunsWithWaitingGates()).toEqual([])
  })

  it('returns runs that contain a waiting node', () => {
    saveWorkflowRun(run('wf-1', 'running', 'waiting'))
    saveWorkflowRun(run('wf-2', 'success', 'success'))
    const result = listRunsWithWaitingGates()
    expect(result).toHaveLength(1)
    expect(result[0].workflowId).toBe('wf-1')
    expect(result[0].nodeStates[0].status).toBe('waiting')
  })

  it('batches node fetch for multiple waiting runs and preserves ordering', () => {
    saveWorkflowRun(run('wf-1', 'running', 'waiting'))
    saveWorkflowRun(run('wf-2', 'running', 'waiting'))
    saveWorkflowRun(run('wf-3', 'running', 'waiting'))
    const result = listRunsWithWaitingGates()
    expect(result.map((r) => r.workflowId).sort()).toEqual(['wf-1', 'wf-2', 'wf-3'])
    for (const r of result) {
      expect(r.nodeStates[0].status).toBe('waiting')
    }
  })

  it('preserves approved_at and agent metadata through the round-trip', () => {
    const exec: WorkflowExecution = {
      runId: 'wf-9:2026-04-20T10:00:00Z',
      workflowId: 'wf-9',
      startedAt: '2026-04-20T10:00:00Z',
      status: 'running',
      nodeStates: [
        {
          nodeId: 'gate',
          status: 'waiting',
          startedAt: '2026-04-20T10:00:00Z',
          completedAt: '2026-04-20T10:00:05Z',
          sessionId: 's1',
          error: 'none',
          logs: 'log line',
          taskId: 't1',
          agentSessionId: 'as1',
          agentType: 'claude',
          projectName: 'p',
          projectPath: '/p',
          approvedAt: '2026-04-20T10:00:10Z'
        }
      ],
      triggerTaskId: 'trig-1',
      completedAt: '2026-04-20T10:00:10Z'
    }
    saveWorkflowRun(exec)
    const [got] = listRunsWithWaitingGates()
    const ns = got.nodeStates[0]
    expect(ns.status).toBe('waiting')
    expect(ns.agentType).toBe('claude')
    expect(ns.projectName).toBe('p')
    expect(ns.projectPath).toBe('/p')
    expect(ns.approvedAt).toBe('2026-04-20T10:00:10Z')
    expect(ns.agentSessionId).toBe('as1')
    expect(ns.taskId).toBe('t1')
    expect(ns.logs).toBe('log line')
    expect(got.completedAt).toBe('2026-04-20T10:00:10Z')
    expect(got.triggerTaskId).toBe('trig-1')
  })
})

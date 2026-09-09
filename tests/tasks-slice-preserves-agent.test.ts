// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'

import { create } from 'zustand'
import { createTasksSlice } from '../src/renderer/stores/tasks-slice'
import type { AppStore } from '../src/renderer/stores/types'
import type { AppConfig, TaskConfig } from '../src/shared/types'

function makeTask(overrides: Partial<TaskConfig> = {}): TaskConfig {
  return {
    id: 't1',
    projectName: 'demo',
    title: 'Do the thing',
    description: '',
    status: 'todo',
    order: 0,
    assignedAgent: 'codex',
    createdAt: '2026-04-15T00:00:00Z',
    updatedAt: '2026-04-15T00:00:00Z',
    ...overrides
  }
}

function makeStore(initialTasks: TaskConfig[]) {
  const saveConfig = vi.fn()
  ;(globalThis as unknown as { window: { api: Record<string, unknown> } }).window = {
    api: { saveConfig, cleanupTaskImages: vi.fn() }
  }

  const baseConfig: AppConfig = {
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark' },
    projects: [],
    workflows: [],
    tasks: initialTasks
  }

  // Minimal slice wrapper — only the tasks slice is under test, so we
  // bolt just enough of AppStore around it for zustand to be happy.
  const store = create<AppStore>()(
    (set, get, api) =>
      ({
        ...(createTasksSlice(set, get, api) as object),
        config: baseConfig,
        // Unused slice fields — TS-satisfying no-ops for this focused test.
        terminals: new Map(),
        headlessSessions: new Map(),
        workflowExecutions: new Map(),
        activeProject: undefined,
        focusedTerminalId: undefined,
        selectedTaskId: null,
        isTaskDialogOpen: false,
        taskDialogInitialStatus: undefined,
        taskStatusFilter: 'all'
      }) as unknown as AppStore
  )

  return { store, saveConfig }
}

describe('tasks-slice — drag-to-in_progress preserves assignedAgent', () => {
  it('updateTask({ status: "in_progress" }) preserves a task\'s assignedAgent', () => {
    const { store } = makeStore([makeTask({ assignedAgent: 'codex' })])

    store.getState().updateTask('t1', { status: 'in_progress' })

    const updated = store.getState().config?.tasks?.find((t) => t.id === 't1')
    expect(updated?.status).toBe('in_progress')
    expect(updated?.assignedAgent).toBe('codex')
  })

  it('updateTask({ status: "in_progress" }) saves the change for the server to act on', () => {
    // Triggers fire in the server now, off the configuration it is handed —
    // so what the window owes is a saved config carrying the new status.
    const { store, saveConfig } = makeStore([makeTask({ assignedAgent: 'codex' })])

    store.getState().updateTask('t1', { status: 'in_progress' })

    const [saved] = saveConfig.mock.calls.at(-1) as [{ tasks: TaskConfig[] }]
    const task = saved.tasks.find((t) => t.id === 't1')
    expect(task?.status).toBe('in_progress')
    expect(task?.assignedAgent).toBe('codex')
  })

  it('startTask(..., sessionId, agentType) DOES overwrite assignedAgent — documents the dangerous path', () => {
    // This is intentional for the workflow-execution path, which calls
    // startTask with the concrete session + agent that actually launched.
    // The drag path must use updateTask instead (see previous test).
    const { store } = makeStore([makeTask({ assignedAgent: 'codex' })])

    store.getState().startTask('t1', 'sess-1', 'claude')

    const updated = store.getState().config?.tasks?.find((t) => t.id === 't1')
    expect(updated?.assignedAgent).toBe('claude')
    expect(updated?.assignedSessionId).toBe('sess-1')
  })
})

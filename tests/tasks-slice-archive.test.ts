// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('../src/renderer/lib/workflow-triggers', () => ({
  fireTaskCreatedTrigger: vi.fn(),
  fireTaskStatusChangedTrigger: vi.fn()
}))

import { create } from 'zustand'
import { createTasksSlice } from '../src/renderer/stores/tasks-slice'
import type { AppStore } from '../src/renderer/stores/types'
import type { AppConfig, TaskConfig } from '../src/shared/types'

function makeTask(overrides: Partial<TaskConfig> = {}): TaskConfig {
  return {
    id: 't1',
    projectName: 'demo',
    title: 'A task',
    description: '',
    status: 'todo',
    order: 0,
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

  const store = create<AppStore>()(
    (set, get, api) =>
      ({
        ...(createTasksSlice(set, get, api) as object),
        config: baseConfig
      }) as unknown as AppStore
  )

  return { store, saveConfig }
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe('tasks-slice — archive', () => {
  it('archives a done task', () => {
    const { store } = makeStore([makeTask({ status: 'done' })])

    store.getState().archiveTask('t1')

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.archivedAt).toBeTruthy()
  })

  it('archives a cancelled task', () => {
    const { store } = makeStore([makeTask({ status: 'cancelled' })])

    store.getState().archiveTask('t1')

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.archivedAt).toBeTruthy()
  })

  it('does not archive an in_progress task', () => {
    const { store, saveConfig } = makeStore([makeTask({ status: 'in_progress' })])

    store.getState().archiveTask('t1')

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.archivedAt).toBeUndefined()
    expect(saveConfig).not.toHaveBeenCalled()
  })

  it('does not archive a todo task', () => {
    const { store, saveConfig } = makeStore([makeTask({ status: 'todo' })])

    store.getState().archiveTask('t1')

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.archivedAt).toBeUndefined()
    expect(saveConfig).not.toHaveBeenCalled()
  })

  it('does not archive an in_review task', () => {
    const { store, saveConfig } = makeStore([makeTask({ status: 'in_review' })])

    store.getState().archiveTask('t1')

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.archivedAt).toBeUndefined()
    expect(saveConfig).not.toHaveBeenCalled()
  })

  it('skips the disk write when the task is already archived', () => {
    const { store, saveConfig } = makeStore([
      makeTask({ status: 'done', archivedAt: '2026-04-20T00:00:00Z' })
    ])

    store.getState().archiveTask('t1')

    expect(saveConfig).not.toHaveBeenCalled()
  })
})

describe('tasks-slice — unarchive', () => {
  it('clears archivedAt on an archived task', () => {
    const { store } = makeStore([makeTask({ status: 'done', archivedAt: '2026-04-20T00:00:00Z' })])

    store.getState().unarchiveTask('t1')

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.archivedAt).toBeUndefined()
  })

  it('is a no-op for a non-archived task', () => {
    const { store, saveConfig } = makeStore([makeTask({ status: 'done' })])

    store.getState().unarchiveTask('t1')

    expect(saveConfig).not.toHaveBeenCalled()
  })
})

describe('tasks-slice — archived clears on status transitions', () => {
  it('reopenTask clears archivedAt', () => {
    const { store } = makeStore([
      makeTask({ status: 'cancelled', archivedAt: '2026-04-20T00:00:00Z' })
    ])

    store.getState().reopenTask('t1')

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.status).toBe('todo')
    expect(t?.archivedAt).toBeUndefined()
  })

  it('updateTask clears archivedAt when status moves to a non-terminal value', () => {
    const { store } = makeStore([makeTask({ status: 'done', archivedAt: '2026-04-20T00:00:00Z' })])

    store.getState().updateTask('t1', { status: 'in_progress' })

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.status).toBe('in_progress')
    expect(t?.archivedAt).toBeUndefined()
  })

  it('updateTask preserves archivedAt when status stays terminal', () => {
    const { store } = makeStore([makeTask({ status: 'done', archivedAt: '2026-04-20T00:00:00Z' })])

    store.getState().updateTask('t1', { status: 'cancelled' })

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.status).toBe('cancelled')
    expect(t?.archivedAt).toBe('2026-04-20T00:00:00Z')
  })

  it('startTask clears archivedAt', () => {
    const { store } = makeStore([
      makeTask({ status: 'cancelled', archivedAt: '2026-04-20T00:00:00Z' })
    ])

    store.getState().startTask('t1', 'sess-1', 'claude')

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.status).toBe('in_progress')
    expect(t?.archivedAt).toBeUndefined()
  })

  it('reviewTask clears archivedAt', () => {
    const { store } = makeStore([makeTask({ status: 'done', archivedAt: '2026-04-20T00:00:00Z' })])

    store.getState().reviewTask('t1')

    const t = store.getState().config?.tasks?.find((x) => x.id === 't1')
    expect(t?.status).toBe('in_review')
    expect(t?.archivedAt).toBeUndefined()
  })
})

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

// Mock logger and filesystem to prevent side effects
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, existsSync: vi.fn(() => true), mkdirSync: vi.fn() }
})

import {
  initTestDatabase,
  dbInsertTask,
  dbUpdateTask,
  dbGetTask,
  dbGetMaxTaskOrder
} from '../packages/server/src/database'
import type { TaskConfig } from '@vornrun/shared/types'

/**
 * Against the real table, because the column list cannot be tested through a mock.
 *
 * `tests/task-write-methods.test.ts` replaces the whole database module, so its
 * idea of which columns `dbUpdateTask` writes is a hand-copied list rather than
 * the function's own. Deleting the `project_name` branch from the real function
 * leaves every one of those tests green -- which is the state this change was
 * made to leave behind, so it is the one thing that has to be checked here.
 */

let teardown: () => void

function task(over: Partial<TaskConfig> = {}): TaskConfig {
  const now = new Date().toISOString()
  return {
    id: `task-${Math.random().toString(36).slice(2)}`,
    projectName: 'alpha',
    title: 'A task',
    description: '',
    status: 'todo',
    order: 0,
    createdAt: now,
    updatedAt: now,
    ...over
  }
}

beforeEach(() => {
  teardown = initTestDatabase()
})

afterEach(() => {
  teardown()
})

describe('moving a task between projects, in the table itself', () => {
  it('writes project_name, which it could not do at all', () => {
    const row = task()
    dbInsertTask(row)

    dbUpdateTask(row.id, { projectName: 'beta' })

    expect(dbGetTask(row.id)?.projectName).toBe('beta')
  })

  it('leaves the project alone when it is not in the update', () => {
    const row = task()
    dbInsertTask(row)

    dbUpdateTask(row.id, { title: 'Renamed' })

    expect(dbGetTask(row.id)?.projectName).toBe('alpha')
    expect(dbGetTask(row.id)?.title).toBe('Renamed')
  })

  it('counts the orders of the board a task has arrived on', () => {
    // What the handler asks before it puts a moved task at the end.
    dbInsertTask(task({ projectName: 'beta', order: 4 }))
    expect(dbGetMaxTaskOrder('beta')).toBe(4)
    expect(dbGetMaxTaskOrder('gamma')).toBe(-1)

    const moving = task({ projectName: 'alpha', order: 0 })
    dbInsertTask(moving)
    dbUpdateTask(moving.id, { projectName: 'beta', order: dbGetMaxTaskOrder('beta') + 1 })

    expect(dbGetTask(moving.id)?.order).toBe(5)
  })
})

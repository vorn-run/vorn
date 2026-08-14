import { describe, it, expect, vi } from 'vitest'

// Mock lucide-react to avoid React dependency
vi.mock('lucide-react', () => ({
  Circle: 'Circle',
  Clock: 'Clock',
  Eye: 'Eye',
  CheckCircle2: 'CheckCircle2',
  XCircle: 'XCircle'
}))

import {
  formatTaskDate,
  getTaskShortId,
  TASK_STATUS_TEXT,
  TASK_STATUS_DOT
} from '../src/renderer/lib/task-status'
import { STATUS_TEXT as SESSION_TEXT } from '../src/renderer/lib/status-colors'
import { WORKFLOW_STATUS_TEXT as WORKFLOW_TEXT } from '../src/renderer/lib/workflow-status'
import type { TaskStatus } from '../src/shared/types'

describe('formatTaskDate', () => {
  it('formats ISO date to month and day', () => {
    const result = formatTaskDate('2025-03-15T12:00:00.000Z')
    expect(result).toMatch(/Mar/)
    expect(result).toMatch(/15/)
  })
})

describe('getTaskShortId', () => {
  it('uses first 3 alpha chars of project name uppercased', () => {
    const result = getTaskShortId({ projectName: 'vorn', id: 'abc12345' })
    expect(result).toBe('VOR-ABC1')
  })

  it('falls back to TSK for non-alpha project name', () => {
    const result = getTaskShortId({ projectName: '123', id: 'xyz99999' })
    expect(result).toBe('TSK-XYZ9')
  })

  it('uses first 4 chars of id uppercased', () => {
    const result = getTaskShortId({ projectName: 'app', id: 'deadbeef' })
    expect(result).toBe('APP-DEAD')
  })
})

describe('task status colour', () => {
  const ALL: TaskStatus[] = ['todo', 'in_progress', 'in_review', 'done', 'cancelled']

  it('accents the one status that wants a person, and only that one', () => {
    // The board carries category colour so a long list can be scanned, but the
    // accent still has to win: in_review is an agent handing work back.
    const accented = ALL.filter((s) => TASK_STATUS_TEXT[s].includes('bronzo'))
    expect(accented).toEqual(['in_review'])
  })

  it('gives the accent status no category colour of its own', () => {
    // Otherwise bronzo becomes just another column tint and stops meaning
    // anything different from slate or sage.
    expect(TASK_STATUS_TEXT.in_review).not.toContain('status-')
    expect(TASK_STATUS_DOT.in_review).not.toContain('status-')
  })

  it('leaves an abandoned task without a colour', () => {
    // Cancelled is an absence, not a category.
    expect(TASK_STATUS_TEXT.cancelled).not.toContain('status-')
    expect(TASK_STATUS_TEXT.cancelled).not.toContain('bronzo')
  })

  it('keeps category colour off every other surface', () => {
    // These exist because a task board is organised by status and read by
    // scanning for one. Sessions and workflows are not, and a category hue
    // there is the thing phases 1 and 2 spent their whole diff removing.
    for (const cls of Object.values(SESSION_TEXT)) expect(cls).not.toContain('status-')
    for (const cls of Object.values(WORKFLOW_TEXT)) expect(cls).not.toContain('status-')
  })

  it('says the same thing as a glyph tint and as a dot', () => {
    for (const s of ALL) {
      expect(TASK_STATUS_TEXT[s].replace(/^text-/, '')).toBe(
        TASK_STATUS_DOT[s].replace(/^bg-/, '').replace('-ghost', '-faint')
      )
    }
  })
})

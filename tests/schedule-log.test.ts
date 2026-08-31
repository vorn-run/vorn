import { describe, it, expect, vi } from 'vitest'

const mockAddEntry = vi.fn<(...args: unknown[]) => unknown>()
const mockGetEntries = vi.fn<(...args: unknown[]) => unknown[]>(() => [])
const mockClear = vi.fn<(...args: unknown[]) => unknown>()

vi.mock('../packages/server/src/database', () => ({
  addScheduleLogEntry: (...args: unknown[]) => mockAddEntry(...args),
  getScheduleLogEntries: (...args: unknown[]) => mockGetEntries(...args),
  clearScheduleLog: (...args: unknown[]) => mockClear(...args)
}))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { scheduleLogManager } from '../packages/server/src/schedule-log'

describe('scheduleLogManager', () => {
  it('addEntry delegates to database', () => {
    const entry = { id: 'e1' } as never
    scheduleLogManager.addEntry(entry)
    expect(mockAddEntry).toHaveBeenCalledWith(entry)
  })

  it('addEntry catches errors silently', () => {
    mockAddEntry.mockImplementationOnce(() => {
      throw new Error('db error')
    })
    expect(() => scheduleLogManager.addEntry({} as never)).not.toThrow()
  })

  it('getEntries returns DB result', () => {
    const data = [{ id: 'e1' }]
    mockGetEntries.mockReturnValueOnce(data)
    expect(scheduleLogManager.getEntries()).toBe(data)
  })

  it('getEntries passes workflowId filter', () => {
    scheduleLogManager.getEntries('wf-1')
    expect(mockGetEntries).toHaveBeenCalledWith('wf-1')
  })

  it('getEntries returns [] on error', () => {
    mockGetEntries.mockImplementationOnce(() => {
      throw new Error('db error')
    })
    expect(scheduleLogManager.getEntries()).toEqual([])
  })

  it('clear delegates to database', () => {
    scheduleLogManager.clear()
    expect(mockClear).toHaveBeenCalled()
  })

  it('clear catches errors silently', () => {
    mockClear.mockImplementationOnce(() => {
      throw new Error('db error')
    })
    expect(() => scheduleLogManager.clear()).not.toThrow()
  })
})

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const mockGetPrevious = vi.fn<(...args: unknown[]) => unknown[]>(() => [])
const mockClearSessions = vi.fn<(...args: unknown[]) => unknown>()

vi.mock('../packages/server/src/database', () => ({
  getPreviousSessions: (...args: unknown[]) => mockGetPrevious(...args),
  clearSessions: (...args: unknown[]) => mockClearSessions(...args)
}))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { sessionManager } from '../packages/server/src/session-persistence'

describe('sessionManager', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    sessionManager.stopAutoSave()
  })

  it('readPreviousSessions returns DB result', () => {
    const data = [{ id: 's1' }]
    mockGetPrevious.mockReturnValueOnce(data)
    expect(sessionManager.readPreviousSessions()).toBe(data)
  })

  it('readPreviousSessions returns null on error, which is not the same as none', () => {
    mockGetPrevious.mockImplementationOnce(() => {
      throw new Error('db error')
    })
    // History is swept when no session claims it, so reading a failure as "there
    // are none" would remove every terminal's history for one bad query.
    expect(sessionManager.readPreviousSessions()).toBeNull()
  })

  it('clear delegates to database', () => {
    sessionManager.clear()
    expect(mockClearSessions).toHaveBeenCalled()
  })

  it('clear catches errors silently', () => {
    mockClearSessions.mockImplementationOnce(() => {
      throw new Error('db error')
    })
    expect(() => sessionManager.clear()).not.toThrow()
  })
})

describe('sessionManager auto-save', () => {
  const mockGetActive = vi.fn(() => [{ id: 's1' }] as never)

  beforeEach(() => {
    vi.useFakeTimers()
    vi.clearAllMocks()
    sessionManager.stopAutoSave()
  })

  afterEach(() => {
    sessionManager.stopAutoSave()
    vi.useRealTimers()
  })

  it('reads nothing before startAutoSave', () => {
    sessionManager.scheduleSave()
    sessionManager.persistNow()
    vi.advanceTimersByTime(1000)
    expect(mockGetActive).not.toHaveBeenCalled()
  })

  it('walks the sessions at once on persistNow, and writes no session record', () => {
    sessionManager.startAutoSave(mockGetActive)
    sessionManager.persistNow()
    expect(mockGetActive).toHaveBeenCalledTimes(1)
    expect(mockClearSessions).not.toHaveBeenCalled()
  })

  it('debounces, so rapid changes cost one walk', () => {
    sessionManager.startAutoSave(mockGetActive)
    sessionManager.scheduleSave()
    sessionManager.scheduleSave()
    sessionManager.scheduleSave()
    expect(mockGetActive).not.toHaveBeenCalled()
    vi.advanceTimersByTime(500)
    expect(mockGetActive).toHaveBeenCalledTimes(1)
  })

  it('stopAutoSave cancels a pending walk', () => {
    sessionManager.startAutoSave(mockGetActive)
    sessionManager.scheduleSave()
    sessionManager.stopAutoSave()
    vi.advanceTimersByTime(30_000)
    expect(mockGetActive).not.toHaveBeenCalled()
  })
})

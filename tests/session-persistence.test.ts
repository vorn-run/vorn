import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const mockSaveSessions = vi.fn<(...args: unknown[]) => unknown>()
const mockGetPrevious = vi.fn<(...args: unknown[]) => unknown[]>(() => [])
const mockClearSessions = vi.fn<(...args: unknown[]) => unknown>()

vi.mock('../packages/server/src/database', () => ({
  saveSessions: (...args: unknown[]) => mockSaveSessions(...args),
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

  it('saveSessions delegates to database', () => {
    const sessions = [{ id: 's1' }] as never
    sessionManager.saveSessions(sessions)
    expect(mockSaveSessions).toHaveBeenCalledWith(sessions)
  })

  it('saveSessions catches errors silently', () => {
    mockSaveSessions.mockImplementationOnce(() => {
      throw new Error('db error')
    })
    expect(() => sessionManager.saveSessions([])).not.toThrow()
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

  it('scheduleSave is a no-op before startAutoSave', () => {
    sessionManager.scheduleSave()
    vi.advanceTimersByTime(1000)
    expect(mockSaveSessions).not.toHaveBeenCalled()
  })

  it('persistNow is a no-op before startAutoSave', () => {
    sessionManager.persistNow()
    expect(mockSaveSessions).not.toHaveBeenCalled()
  })

  it('persistNow saves immediately after startAutoSave', () => {
    sessionManager.startAutoSave(mockGetActive)
    sessionManager.persistNow()
    expect(mockGetActive).toHaveBeenCalled()
    expect(mockSaveSessions).toHaveBeenCalledWith([{ id: 's1' }])
  })

  it('scheduleSave debounces — rapid calls produce one save', () => {
    sessionManager.startAutoSave(mockGetActive)
    sessionManager.scheduleSave()
    sessionManager.scheduleSave()
    sessionManager.scheduleSave()
    // Before debounce fires
    expect(mockSaveSessions).not.toHaveBeenCalled()
    // After debounce (500ms)
    vi.advanceTimersByTime(500)
    expect(mockSaveSessions).toHaveBeenCalledTimes(1)
  })

  it('periodic interval saves only when dirty', () => {
    sessionManager.startAutoSave(mockGetActive)
    // Advance 30s — not dirty, should not save
    vi.advanceTimersByTime(30_000)
    expect(mockSaveSessions).not.toHaveBeenCalled()

    // Mark dirty via scheduleSave, but clear it via persistNow before interval
    sessionManager.scheduleSave()
    vi.advanceTimersByTime(500) // debounce fires, clears dirty
    mockSaveSessions.mockClear()

    // Next interval tick — dirty is false again, should not save
    vi.advanceTimersByTime(30_000)
    expect(mockSaveSessions).not.toHaveBeenCalled()
  })

  it('periodic interval saves when dirty flag is set', () => {
    sessionManager.startAutoSave(mockGetActive)
    sessionManager.scheduleSave()
    // Don't let debounce fire — advance straight to interval
    vi.advanceTimersByTime(30_000)
    // Both debounce (at 500ms) and interval (at 30s) should have fired
    expect(mockSaveSessions).toHaveBeenCalled()
  })

  it('stopAutoSave prevents further saves', () => {
    sessionManager.startAutoSave(mockGetActive)
    sessionManager.scheduleSave()
    sessionManager.stopAutoSave()
    vi.advanceTimersByTime(30_000)
    expect(mockSaveSessions).not.toHaveBeenCalled()
  })
})

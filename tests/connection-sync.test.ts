// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { waitForSync } from '../src/renderer/lib/connection-sync'

const listConnections = vi.fn()

beforeEach(() => {
  vi.useFakeTimers()
  listConnections.mockReset()
  vi.stubGlobal('api', { listConnections })
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

/** Lets the awaited reads between two ticks settle. */
const tick = async (ms: number) => {
  await vi.advanceTimersByTimeAsync(ms)
}

describe('waiting for the sync a manual run started', () => {
  it('returns once the connection reports a later sync than it had', async () => {
    listConnections
      .mockResolvedValueOnce([{ id: 'c1', lastSyncAt: '2026-09-04T10:00:00.000Z' }])
      .mockResolvedValue([{ id: 'c1', lastSyncAt: '2026-09-04T10:05:00.000Z' }])
    let done = false

    void waitForSync('c1', '2026-09-04T10:00:00.000Z').then(() => {
      done = true
    })

    await tick(500)
    expect(done).toBe(false)
    await tick(500)
    expect(done).toBe(true)
    expect(listConnections).toHaveBeenCalledTimes(2)
  })

  it('returns on a first sync, when the connection had none', async () => {
    listConnections.mockResolvedValue([{ id: 'c1', lastSyncAt: '2026-09-04T10:00:00.000Z' }])
    let done = false

    void waitForSync('c1', undefined).then(() => {
      done = true
    })

    await tick(500)
    expect(done).toBe(true)
  })

  it('gives up rather than watching forever', async () => {
    listConnections.mockResolvedValue([{ id: 'c1', lastSyncAt: '2026-09-04T10:00:00.000Z' }])
    let done = false

    void waitForSync('c1', '2026-09-04T10:00:00.000Z').then(() => {
      done = true
    })

    await tick(14_000)
    expect(done).toBe(false)
    await tick(2_000)
    expect(done).toBe(true)
  })
})

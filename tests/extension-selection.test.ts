import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { IPC } from '@vornrun/shared/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

const broadcasts: Array<{ method: string; params: unknown; scope?: string }> = []
const registry = { size: 1 }

vi.mock('../packages/server/src/broadcast', () => ({
  clientRegistry: {
    broadcast: (method: string, params: unknown, scope?: string) =>
      broadcasts.push({ method, params, scope }),
    get size() {
      return registry.size
    }
  }
}))

const selection = await import('../packages/server/src/extensions/selection')

beforeEach(() => {
  broadcasts.length = 0
  registry.size = 1
})

afterEach(() => {
  selection.abandonSelections()
  vi.useRealTimers()
})

describe('asking a window what is selected', () => {
  it('asks the windows drawing that session, and takes the answer', async () => {
    const asked = selection.requestSelection('s1')
    expect(broadcasts).toHaveLength(1)
    expect(broadcasts[0].method).toBe(IPC.EXTENSION_SELECTION_REQUEST)
    expect(broadcasts[0].scope).toBe('s1')

    const { requestId } = broadcasts[0].params as { requestId: number }
    selection.resolveSelection(requestId, 'const a = 1')
    expect(await asked).toBe('const a = 1')
  })

  // A window that cannot say is a window with nothing selected; a footer must not hang on it.
  it('reads silence as no selection', async () => {
    vi.useFakeTimers()
    const asked = selection.requestSelection('s1')
    await vi.advanceTimersByTimeAsync(15_000)
    expect(await asked).toBe('')
  })

  it('answers nothing at all when no window is connected', async () => {
    registry.size = 0
    expect(await selection.requestSelection('s1')).toBe('')
    expect(broadcasts).toHaveLength(0)
  })

  it('takes the first answer, and ignores the rest', async () => {
    const asked = selection.requestSelection('s1')
    const { requestId } = broadcasts[0].params as { requestId: number }
    selection.resolveSelection(requestId, 'first')
    selection.resolveSelection(requestId, 'second')
    expect(await asked).toBe('first')
  })

  it('matches an answer to the request that asked for it', async () => {
    const first = selection.requestSelection('s1')
    const second = selection.requestSelection('s2')
    const ids = broadcasts.map((one) => (one.params as { requestId: number }).requestId)
    selection.resolveSelection(ids[1], 'from the second')
    selection.resolveSelection(ids[0], 'from the first')
    expect(await first).toBe('from the first')
    expect(await second).toBe('from the second')
  })
})

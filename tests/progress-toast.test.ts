// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'

const mockLoading = vi.fn((_msg: string) => 'toast-id-1')
const mockUpdate = vi.fn<(...args: unknown[]) => unknown>()

vi.mock('../src/renderer/components/Toast', () => ({
  toast: {
    loading: (msg: string) => mockLoading(msg),
    update: (id: string, msg: string, type: string) => mockUpdate(id, msg, type)
  }
}))

import { withProgressToast } from '../src/renderer/lib/progress-toast'

describe('withProgressToast', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockLoading.mockReturnValue('toast-id-1')
  })

  it('shows loading toast immediately with the loading label', async () => {
    await withProgressToast({ loading: 'Loading…', success: 'Done' }, async () => 'value')
    expect(mockLoading).toHaveBeenCalledWith('Loading…')
  })

  it('transitions toast to success with the success label', async () => {
    await withProgressToast({ loading: 'Loading…', success: 'Done' }, async () => 'value')
    expect(mockUpdate).toHaveBeenCalledWith('toast-id-1', 'Done', 'success')
  })

  it('returns the resolved value on success', async () => {
    const result = await withProgressToast({ loading: 'Loading…', success: 'Done' }, async () => 42)
    expect(result).toBe(42)
  })

  it('transitions toast to error with the Error message on failure', async () => {
    await withProgressToast({ loading: 'Loading…', success: 'Done' }, async () => {
      throw new Error('Something broke')
    })
    expect(mockUpdate).toHaveBeenCalledWith('toast-id-1', 'Something broke', 'error')
  })

  it('returns undefined on failure instead of throwing', async () => {
    const result = await withProgressToast({ loading: 'Loading…', success: 'Done' }, async () => {
      throw new Error('fail')
    })
    expect(result).toBeUndefined()
  })

  it('coerces non-Error throws to string', async () => {
    await withProgressToast({ loading: 'Loading…', success: 'Done' }, async () =>
      Promise.reject('plain string')
    )
    expect(mockUpdate).toHaveBeenCalledWith('toast-id-1', 'plain string', 'error')
  })

  it('uses custom error formatter when provided', async () => {
    await withProgressToast(
      {
        loading: 'Loading…',
        success: 'Done',
        error: (err) => `Custom: ${(err as Error).message}`
      },
      async () => {
        throw new Error('boom')
      }
    )
    expect(mockUpdate).toHaveBeenCalledWith('toast-id-1', 'Custom: boom', 'error')
  })

  it('falls back to "Operation failed" when error message is empty', async () => {
    await withProgressToast({ loading: 'Loading…', success: 'Done' }, async () => {
      throw new Error('')
    })
    expect(mockUpdate).toHaveBeenCalledWith('toast-id-1', 'Operation failed', 'error')
  })

  it('does not call update on success with the loading id twice', async () => {
    await withProgressToast({ loading: 'Loading…', success: 'Done' }, async () => 'v')
    expect(mockUpdate).toHaveBeenCalledTimes(1)
  })
})

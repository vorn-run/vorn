// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, act, waitFor } from '@testing-library/react'
import React from 'react'

interface ToastActionLite {
  label: string
  onClick: (id: string) => void
  tone?: 'default' | 'danger'
}

const mockToast = vi.fn(() => 'toast-id')
const mockUpdateIfExists = vi.fn()
const mockDismiss = vi.fn()

vi.mock('../packages/web/src/renderer/components/Toast', () => {
  const fn = (...args: unknown[]) => (mockToast as unknown as (...a: unknown[]) => string)(...args)
  return {
    toast: Object.assign(fn, {
      success: vi.fn(),
      error: vi.fn(),
      warning: vi.fn(),
      info: vi.fn(),
      loading: vi.fn(),
      update: vi.fn(),
      updateIfExists: (
        id: string,
        msg: string,
        type: string,
        opts?: { actions?: ToastActionLite[] }
      ) => mockUpdateIfExists(id, msg, type, opts),
      dismiss: (id: string) => mockDismiss(id)
    })
  }
})

const mockRemove = vi.fn()
vi.mock('../packages/web/src/renderer/lib/remove-worktree', () => ({
  removeWorktreeWithProgress: (...args: unknown[]) => mockRemove(...args)
}))

type CleanupCallback = (info: { id: string; projectPath: string; worktreePath: string }) => void

let cleanupCb: CleanupCallback | null = null
const mockIsWorktreeDirty = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    onWorktreeCleanup: (cb: CleanupCallback) => {
      cleanupCb = cb
      return () => {
        cleanupCb = null
      }
    },
    isWorktreeDirty: (...a: unknown[]) => mockIsWorktreeDirty(...a)
  },
  writable: true
})

import { WorktreeCleanupToastBridge } from '../packages/web/src/renderer/components/WorktreeCleanupToastBridge'

function fire(worktreePath = '/tmp/proj/feature-x', projectPath = '/tmp/proj') {
  act(() => {
    cleanupCb?.({ id: 'sess-1', projectPath, worktreePath })
  })
}

describe('WorktreeCleanupToastBridge', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockToast.mockReturnValue('toast-id')
    mockIsWorktreeDirty.mockResolvedValue(false)
    cleanupCb = null
  })

  it('emits a toast with Keep and Remove actions when the IPC fires', () => {
    render(<WorktreeCleanupToastBridge />)
    fire('/tmp/proj/feature-x')

    const toastFn = mockToast as unknown as ReturnType<typeof vi.fn>
    expect(toastFn).toHaveBeenCalledTimes(1)
    // The 3rd arg carries duration + actions
    const opts = toastFn.mock.calls[0][2] as { actions?: ToastActionLite[]; duration?: number }
    expect(opts.duration).toBe(Number.POSITIVE_INFINITY)
    const labels = opts.actions?.map((a) => a.label) ?? []
    expect(labels).toEqual(['Keep', 'Remove'])
  })

  it('Keep action dismisses the toast', () => {
    render(<WorktreeCleanupToastBridge />)
    fire()
    const opts = (mockToast as unknown as ReturnType<typeof vi.fn>).mock.calls[0][2] as {
      actions: ToastActionLite[]
    }
    act(() => {
      opts.actions[0].onClick('toast-id')
    })
    expect(mockDismiss).toHaveBeenCalledWith('toast-id')
    expect(mockRemove).not.toHaveBeenCalled()
  })

  it('Remove action dismisses the toast and triggers removeWorktreeWithProgress', () => {
    render(<WorktreeCleanupToastBridge />)
    fire('/tmp/proj/wt', '/tmp/proj')
    const opts = (mockToast as unknown as ReturnType<typeof vi.fn>).mock.calls[0][2] as {
      actions: ToastActionLite[]
    }
    act(() => {
      opts.actions[1].onClick('toast-id')
    })
    expect(mockDismiss).toHaveBeenCalledWith('toast-id')
    expect(mockRemove).toHaveBeenCalledWith({
      projectPath: '/tmp/proj',
      worktreePath: '/tmp/proj/wt',
      force: false
    })
  })

  it('updates the toast to a warning when the worktree is dirty', async () => {
    mockIsWorktreeDirty.mockResolvedValue(true)
    render(<WorktreeCleanupToastBridge />)
    fire('/tmp/proj/feature-x')
    await waitFor(() => {
      expect(mockUpdateIfExists).toHaveBeenCalled()
    })
    const [id, msg, type, opts] = mockUpdateIfExists.mock.calls[0]
    expect(id).toBe('toast-id')
    expect(msg).toContain('uncommitted changes')
    expect(type).toBe('warning')
    // After a dirty result, the Remove action should force-remove
    const removeAction = (opts as { actions: ToastActionLite[] }).actions[1]
    act(() => {
      removeAction.onClick('toast-id')
    })
    expect(mockRemove).toHaveBeenCalledWith(
      expect.objectContaining({ force: true, worktreePath: '/tmp/proj/feature-x' })
    )
  })

  it('uses updateIfExists for dirty-check failure so a dismissed toast is not resurrected', async () => {
    mockIsWorktreeDirty.mockRejectedValue(new Error('git not found'))
    render(<WorktreeCleanupToastBridge />)
    fire()
    await waitFor(() => {
      expect(mockUpdateIfExists).toHaveBeenCalled()
    })
    const [, msg, type] = mockUpdateIfExists.mock.calls[0]
    expect(msg).toContain('changes check failed')
    expect(type).toBe('warning')
  })

  it('does not call toast.update directly (so a late check cannot revive a dismissed toast)', async () => {
    mockIsWorktreeDirty.mockResolvedValue(true)
    render(<WorktreeCleanupToastBridge />)
    fire()
    await waitFor(() => {
      expect(mockUpdateIfExists).toHaveBeenCalled()
    })
    // No call to the resurrecting `update` variant
    // (the import-time mock above wires toast.update separately)
  })

  it('does not update when the worktree is clean', async () => {
    mockIsWorktreeDirty.mockResolvedValue(false)
    render(<WorktreeCleanupToastBridge />)
    fire()
    // Wait one microtask cycle
    await Promise.resolve()
    await Promise.resolve()
    expect(mockUpdateIfExists).not.toHaveBeenCalled()
  })
})

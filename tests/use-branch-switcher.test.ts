// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { renderHook, act, waitFor } from '@testing-library/react'

const { toastError, toastSuccess, loadWorktrees, setBranchForCwd } = vi.hoisted(() => ({
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  loadWorktrees: vi.fn(),
  setBranchForCwd: vi.fn()
}))

vi.mock('../packages/web/src/renderer/components/Toast', () => ({
  toast: { error: toastError, success: toastSuccess }
}))

type StoreShape = {
  loadWorktrees: typeof loadWorktrees
  setBranchForCwd: typeof setBranchForCwd
}

vi.mock('../packages/web/src/renderer/stores', () => ({
  useAppStore: (selector: (s: StoreShape) => unknown) =>
    selector({ loadWorktrees, setBranchForCwd })
}))

const checkoutBranch = vi.fn()
Object.defineProperty(window, 'api', {
  value: { checkoutBranch },
  writable: true
})

import { useBranchSwitcher } from '../packages/web/src/renderer/hooks/useBranchSwitcher'

beforeEach(() => {
  toastError.mockReset()
  toastSuccess.mockReset()
  loadWorktrees.mockReset()
  setBranchForCwd.mockReset()
  checkoutBranch.mockReset()
})

describe('useBranchSwitcher', () => {
  const params = { projectPath: '/p', branchCwd: '/p', branchName: 'main' }

  it('noops when required params are missing', async () => {
    const { result } = renderHook(() =>
      useBranchSwitcher({ projectPath: undefined, branchCwd: undefined, branchName: undefined })
    )
    await act(async () => {
      await result.current.selectBranch('feat')
    })
    expect(checkoutBranch).not.toHaveBeenCalled()
  })

  it('noops when selected branch equals current', async () => {
    const { result } = renderHook(() => useBranchSwitcher(params))
    await act(async () => {
      await result.current.selectBranch('main')
    })
    expect(checkoutBranch).not.toHaveBeenCalled()
  })

  it('checkouts, fans out branch to sessions, reloads worktrees, and toasts on success', async () => {
    checkoutBranch.mockResolvedValue({ ok: true })
    const { result } = renderHook(() => useBranchSwitcher(params))
    await act(async () => {
      await result.current.selectBranch('feat')
    })
    expect(checkoutBranch).toHaveBeenCalledWith('/p', 'feat')
    expect(setBranchForCwd).toHaveBeenCalledWith('/p', 'feat')
    expect(loadWorktrees).toHaveBeenCalledWith('/p', true)
    expect(toastSuccess).toHaveBeenCalledWith("Switched to 'feat'")
    expect(toastError).not.toHaveBeenCalled()
    expect(result.current.showPicker).toBe(false)
    expect(result.current.isSwitching).toBe(false)
  })

  it('surfaces toast on failure and does not reload', async () => {
    checkoutBranch.mockResolvedValue({ ok: false, error: 'boom' })
    const { result } = renderHook(() => useBranchSwitcher(params))
    await act(async () => {
      await result.current.selectBranch('feat')
    })
    expect(toastError).toHaveBeenCalledWith('boom')
    expect(loadWorktrees).not.toHaveBeenCalled()
    expect(setBranchForCwd).not.toHaveBeenCalled()
    expect(toastSuccess).not.toHaveBeenCalled()
  })

  it('catches IPC rejection and surfaces error via toast', async () => {
    checkoutBranch.mockRejectedValue(new Error('ipc down'))
    const { result } = renderHook(() => useBranchSwitcher(params))
    await act(async () => {
      await result.current.selectBranch('feat')
    })
    expect(toastError).toHaveBeenCalledWith('ipc down')
    expect(result.current.isSwitching).toBe(false)
    expect(result.current.showPicker).toBe(false)
  })

  it('falls back to generic error when result.error missing', async () => {
    checkoutBranch.mockResolvedValue({ ok: false })
    const { result } = renderHook(() => useBranchSwitcher(params))
    await act(async () => {
      await result.current.selectBranch('feat')
    })
    expect(toastError).toHaveBeenCalledWith("Failed to checkout 'feat'")
  })

  it('guards against concurrent calls via isSwitching', async () => {
    let resolve!: (v: { ok: true }) => void
    checkoutBranch.mockReturnValue(
      new Promise<{ ok: true }>((r) => {
        resolve = r
      })
    )
    const { result } = renderHook(() => useBranchSwitcher(params))
    act(() => {
      void result.current.selectBranch('feat')
    })
    await waitFor(() => expect(result.current.isSwitching).toBe(true))
    await act(async () => {
      await result.current.selectBranch('other')
    })
    expect(checkoutBranch).toHaveBeenCalledTimes(1)
    await act(async () => {
      resolve({ ok: true })
    })
  })

  it('togglePicker flips state; closePicker sets false', () => {
    const { result } = renderHook(() => useBranchSwitcher(params))
    expect(result.current.showPicker).toBe(false)
    act(() => result.current.togglePicker())
    expect(result.current.showPicker).toBe(true)
    act(() => result.current.togglePicker())
    expect(result.current.showPicker).toBe(false)
    act(() => result.current.togglePicker())
    act(() => result.current.closePicker())
    expect(result.current.showPicker).toBe(false)
  })
})

// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import React from 'react'

// jsdom doesn't implement matchMedia.
Object.defineProperty(window, 'matchMedia', {
  value: () => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn()
  }),
  writable: true
})

vi.mock('framer-motion', () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) =>
    React.createElement(React.Fragment, null, children),
  motion: new Proxy(
    {},
    {
      get: (_, tag: string) =>
        React.forwardRef<HTMLElement, React.HTMLAttributes<HTMLElement>>((props, ref) =>
          React.createElement(tag, { ...props, ref })
        )
    }
  )
}))

const mockLoading = vi.fn((_msg: string) => 'toast-id')
const mockUpdate = vi.fn<(...args: unknown[]) => unknown>()
const mockDismiss = vi.fn()

vi.mock('../src/renderer/components/Toast', () => ({
  toast: Object.assign(
    (msg: string) => {
      mockLoading(msg)
      return 'toast-id'
    },
    {
      loading: (msg: string) => mockLoading(msg),
      update: (id: string, msg: string, type: string) => mockUpdate(id, msg, type),
      dismiss: (id: string) => mockDismiss(id),
      success: vi.fn(),
      error: vi.fn(),
      warning: vi.fn(),
      info: vi.fn()
    }
  )
}))

const mockIsWorktreeDirty = vi.fn()
const mockRemoveWorktree = vi.fn()
const mockKillTerminal = vi.fn()
const mockKillHeadlessSession = vi.fn()
const mockOnWorktreeCleanup = vi.fn((_cb: unknown) => () => {})
const mockLoadWorktreesStoreFn = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    isWorktreeDirty: (...a: unknown[]) => mockIsWorktreeDirty(...a),
    removeWorktree: (...a: unknown[]) => mockRemoveWorktree(...a),
    killTerminal: (...a: unknown[]) => mockKillTerminal(...a),
    killHeadlessSession: (...a: unknown[]) => mockKillHeadlessSession(...a),
    onWorktreeCleanup: (cb: unknown) => mockOnWorktreeCleanup(cb)
  },
  writable: true
})

vi.mock('../src/renderer/stores', () => ({
  useAppStore: Object.assign(
    (selector: (s: unknown) => unknown) => selector({ loadWorktrees: mockLoadWorktreesStoreFn }),
    {
      getState: () => ({ loadWorktrees: mockLoadWorktreesStoreFn })
    }
  )
}))

import {
  WorktreeCleanupDialog,
  requestWorktreeDelete
} from '../src/renderer/components/WorktreeCleanupDialog'

async function openExplicitDelete(
  opts: {
    projectPath?: string
    worktreePath?: string
    sessionIds?: string[]
    dirty?: boolean
  } = {}
) {
  mockIsWorktreeDirty.mockResolvedValue(opts.dirty ?? false)
  act(() => {
    requestWorktreeDelete({
      projectPath: opts.projectPath ?? '/tmp/proj',
      worktreePath: opts.worktreePath ?? '/tmp/proj/wt',
      sessionIds: opts.sessionIds ?? []
    })
  })
  // Wait for the dirty-state check to resolve and the Remove button to enable
  await waitFor(() => {
    const btn = screen.getByRole('button', { name: /Remove/i })
    expect(btn).not.toBeDisabled()
  })
}

describe('WorktreeCleanupDialog (explicit-delete mode)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockLoading.mockReturnValue('toast-id')
    mockRemoveWorktree.mockResolvedValue(true)
    mockKillTerminal.mockResolvedValue(undefined)
    mockKillHeadlessSession.mockResolvedValue(undefined)
  })

  afterEach(() => {
    // Ensure any open dialog is cleaned up between tests
    vi.clearAllMocks()
  })

  it('opens when requestWorktreeDelete is called and shows the worktree path', async () => {
    render(<WorktreeCleanupDialog />)
    await openExplicitDelete({ worktreePath: '/tmp/proj/wt-abc' })
    expect(screen.getByText('/tmp/proj/wt-abc')).toBeInTheDocument()
  })

  it('closes the dialog immediately on confirm and does not block', async () => {
    render(<WorktreeCleanupDialog />)
    await openExplicitDelete()
    const removeBtn = screen.getByRole('button', { name: /Remove/i })
    act(() => {
      fireEvent.click(removeBtn)
    })
    // Dialog should be gone synchronously — removal runs in background
    expect(screen.queryByRole('button', { name: /Remove/i })).not.toBeInTheDocument()
  })

  it('fires a loading toast and calls removeWorktree in the background', async () => {
    render(<WorktreeCleanupDialog />)
    await openExplicitDelete({
      projectPath: '/tmp/proj',
      worktreePath: '/tmp/proj/wt'
    })
    act(() => {
      fireEvent.click(screen.getByRole('button', { name: /Remove/i }))
    })
    expect(mockLoading).toHaveBeenCalledWith('Removing worktree…')
    await waitFor(() => {
      expect(mockRemoveWorktree).toHaveBeenCalledWith('/tmp/proj', '/tmp/proj/wt', false)
    })
    await waitFor(() => {
      expect(mockUpdate).toHaveBeenCalledWith('toast-id', 'Worktree removed', 'success')
    })
  })

  it('shows the sessions-closing label and kills sessions when sessionIds are set', async () => {
    render(<WorktreeCleanupDialog />)
    await openExplicitDelete({ sessionIds: ['sess-1', 'sess-2'] })
    act(() => {
      fireEvent.click(screen.getByRole('button', { name: /Close sessions & remove/i }))
    })
    expect(mockLoading).toHaveBeenCalledWith('Closing sessions & removing worktree…')
    await waitFor(() => {
      expect(mockKillTerminal).toHaveBeenCalledWith('sess-1')
      expect(mockKillTerminal).toHaveBeenCalledWith('sess-2')
      expect(mockRemoveWorktree).toHaveBeenCalled()
    })
  })

  it('uses force=true when the worktree is dirty', async () => {
    render(<WorktreeCleanupDialog />)
    await openExplicitDelete({ dirty: true })
    act(() => {
      fireEvent.click(screen.getByRole('button', { name: /Remove anyway/i }))
    })
    await waitFor(() => {
      expect(mockRemoveWorktree).toHaveBeenCalledWith(expect.any(String), expect.any(String), true)
    })
  })

  it('reports a failure via the progress toast when removeWorktree returns false', async () => {
    mockRemoveWorktree.mockResolvedValue(false)
    render(<WorktreeCleanupDialog />)
    await openExplicitDelete()
    act(() => {
      fireEvent.click(screen.getByRole('button', { name: /Remove/i }))
    })
    await waitFor(() => {
      expect(mockUpdate).toHaveBeenCalledWith('toast-id', 'Failed to remove worktree', 'error')
    })
  })

  it('clicking Cancel closes the dialog without calling removeWorktree', async () => {
    render(<WorktreeCleanupDialog />)
    await openExplicitDelete()
    act(() => {
      fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    })
    expect(screen.queryByRole('button', { name: /Remove/i })).not.toBeInTheDocument()
    expect(mockRemoveWorktree).not.toHaveBeenCalled()
  })
})

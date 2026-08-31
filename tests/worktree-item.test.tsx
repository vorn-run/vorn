// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, fireEvent, waitFor, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import React from 'react'

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
const mockToastError = vi.fn()

vi.mock('../src/renderer/components/Toast', () => ({
  toast: Object.assign(
    (msg: string) => {
      mockLoading(msg)
      return 'toast-id'
    },
    {
      loading: (msg: string) => mockLoading(msg),
      update: (id: string, msg: string, type: string) => mockUpdate(id, msg, type),
      dismiss: vi.fn(),
      success: vi.fn(),
      error: (msg: string) => mockToastError(msg),
      warning: vi.fn(),
      info: vi.fn()
    }
  )
}))

const mockCreateSession = vi.fn()
const mockCreateShell = vi.fn().mockResolvedValue(undefined)

vi.mock('../src/renderer/lib/session-utils', () => ({
  createSessionFromProject: (...args: unknown[]) => mockCreateSession(...args),
  createShellInProject: (...args: unknown[]) => mockCreateShell(...args)
}))

const mockRequestWorktreeDelete = vi.fn()

vi.mock('../src/renderer/components/WorktreeCleanupDialog', () => ({
  requestWorktreeDelete: (info: unknown) => mockRequestWorktreeDelete(info),
  WorktreeCleanupDialog: () => null
}))

const mockGetActiveSessions = vi.fn()
const mockRemoveWorktree = vi.fn()
const mockRenameWorktree = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    getWorktreeActiveSessions: (...a: unknown[]) => mockGetActiveSessions(...a),
    removeWorktree: (...a: unknown[]) => mockRemoveWorktree(...a),
    renameWorktree: (...a: unknown[]) => mockRenameWorktree(...a)
  },
  writable: true
})

import { useAppStore } from '../src/renderer/stores'
import { WorktreeItem } from '../src/renderer/components/project-sidebar/WorktreeItem'
import type { ProjectConfig, AppConfig } from '../src/shared/types'
import type { WorktreeInfo } from '../src/renderer/stores/types'

const project: ProjectConfig = {
  name: 'test-proj',
  path: '/tmp/test-proj',
  preferredAgents: []
}

const worktree: WorktreeInfo = {
  path: '/tmp/test-proj/wt-a',
  branch: 'feature-a',
  name: 'feature-a',
  isMain: false,
  isDirty: false
}

const baseConfig: Partial<AppConfig> = {
  projects: [project],
  defaults: { defaultAgent: 'claude' } as AppConfig['defaults'],
  remoteHosts: []
}

const initialState = useAppStore.getState()

function renderWorktreeItem(
  wt: WorktreeInfo = worktree,
  onWorktreesChanged: () => void = vi.fn(),
  overrides: Partial<React.ComponentProps<typeof WorktreeItem>> = {}
) {
  return render(
    <WorktreeItem
      worktree={wt}
      projectPath={project.path}
      projectName={project.name}
      isActiveWorktree={false}
      sessionCount={0}
      onSelect={vi.fn()}
      onWorktreesChanged={onWorktreesChanged}
      {...overrides}
    />
  )
}

describe('WorktreeItem progress-toast handlers', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockCreateSession.mockResolvedValue(undefined)
    mockGetActiveSessions.mockResolvedValue({ count: 0, sessionIds: [] })
    mockRemoveWorktree.mockResolvedValue(true)
    useAppStore.setState({ config: baseConfig as AppConfig })
  })

  afterEach(() => {
    useAppStore.setState(initialState)
  })

  it('new session button fires loading toast and calls createSessionFromProject', async () => {
    const { container } = renderWorktreeItem()
    const sessionBtn = container.querySelector('button[aria-label="New session"]') as HTMLElement
    act(() => {
      fireEvent.click(sessionBtn)
    })
    expect(mockLoading).toHaveBeenCalledWith('Starting session…')
    await waitFor(() => {
      expect(mockCreateSession).toHaveBeenCalledWith(project, {
        branch: 'feature-a',
        existingWorktreePath: '/tmp/test-proj/wt-a'
      })
      expect(mockUpdate).toHaveBeenCalledWith('toast-id', 'Session started', 'success')
    })
  })

  it('new terminal button calls createShellInProject with worktree path', async () => {
    const { container } = renderWorktreeItem()
    const terminalBtn = container.querySelector('button[aria-label="New terminal"]') as HTMLElement
    expect(terminalBtn).not.toBeNull()
    act(() => {
      fireEvent.click(terminalBtn)
    })
    await waitFor(() => {
      expect(mockCreateShell).toHaveBeenCalledWith(
        worktree.path,
        expect.objectContaining({ worktreePath: worktree.path })
      )
    })
  })

  it('new terminal ref-lock prevents synchronous double-click from firing twice', async () => {
    let resolveIt: () => void
    mockCreateShell.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveIt = resolve
        })
    )
    const { container } = renderWorktreeItem()
    const terminalBtn = container.querySelector('button[aria-label="New terminal"]') as HTMLElement
    act(() => {
      fireEvent.click(terminalBtn)
      fireEvent.click(terminalBtn)
    })
    expect(mockCreateShell).toHaveBeenCalledTimes(1)
    act(() => resolveIt!())
  })

  it('new session ref-lock prevents a synchronous double-click from firing twice', async () => {
    let resolveIt: () => void
    mockCreateSession.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveIt = resolve
        })
    )
    const { container } = renderWorktreeItem()
    const sessionBtn = container.querySelector('button[aria-label="New session"]') as HTMLElement
    act(() => {
      fireEvent.click(sessionBtn)
      fireEvent.click(sessionBtn)
    })
    expect(mockCreateSession).toHaveBeenCalledTimes(1)
    act(() => resolveIt!())
    await waitFor(() => expect(mockUpdate).toHaveBeenCalled())
  })

  it('remove button takes the fast path when worktree is clean and has no sessions', async () => {
    const onWorktreesChanged = vi.fn()
    const { container } = renderWorktreeItem(worktree, onWorktreesChanged)
    const buttons = Array.from(container.querySelectorAll('button[type="button"]'))
    const removeBtn = buttons[buttons.length - 1]
    act(() => {
      fireEvent.click(removeBtn)
    })
    await waitFor(() => {
      expect(mockGetActiveSessions).toHaveBeenCalledWith(worktree.path)
    })
    await waitFor(() => {
      expect(mockLoading).toHaveBeenCalledWith('Removing worktree…')
      expect(mockRemoveWorktree).toHaveBeenCalledWith(project.path, worktree.path, false)
      expect(onWorktreesChanged).toHaveBeenCalled()
      expect(mockUpdate).toHaveBeenCalledWith('toast-id', 'Worktree removed', 'success')
    })
  })

  it('remove button routes through cleanup dialog when worktree has active sessions', async () => {
    mockGetActiveSessions.mockResolvedValue({ count: 2, sessionIds: ['s1', 's2'] })
    const { container } = renderWorktreeItem()
    const buttons = Array.from(container.querySelectorAll('button[type="button"]'))
    const removeBtn = buttons[buttons.length - 1]
    act(() => {
      fireEvent.click(removeBtn)
    })
    await waitFor(() => {
      expect(mockRequestWorktreeDelete).toHaveBeenCalledWith({
        projectPath: project.path,
        worktreePath: worktree.path,
        sessionIds: ['s1', 's2']
      })
    })
    // Fast path should NOT run
    expect(mockRemoveWorktree).not.toHaveBeenCalled()
    expect(mockLoading).not.toHaveBeenCalledWith('Removing worktree…')
  })

  it('remove button routes through cleanup dialog when worktree is dirty', async () => {
    const dirty: WorktreeInfo = { ...worktree, isDirty: true }
    const { container } = renderWorktreeItem(dirty)
    const buttons = Array.from(container.querySelectorAll('button[type="button"]'))
    const removeBtn = buttons[buttons.length - 1]
    act(() => {
      fireEvent.click(removeBtn)
    })
    await waitFor(() => {
      expect(mockRequestWorktreeDelete).toHaveBeenCalled()
    })
    expect(mockRemoveWorktree).not.toHaveBeenCalled()
  })

  it('remove fast path transitions toast to error when removeWorktree returns false', async () => {
    mockRemoveWorktree.mockResolvedValue(false)
    const { container } = renderWorktreeItem()
    const buttons = Array.from(container.querySelectorAll('button[type="button"]'))
    const removeBtn = buttons[buttons.length - 1]
    act(() => {
      fireEvent.click(removeBtn)
    })
    await waitFor(() => {
      expect(mockUpdate).toHaveBeenCalledWith('toast-id', 'Failed to remove worktree', 'error')
    })
  })
})

describe('WorktreeItem sessions toggle', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    useAppStore.setState({ config: baseConfig as AppConfig })
  })

  afterEach(() => {
    useAppStore.setState(initialState)
  })

  it('offers no toggle when there is nothing to expand', () => {
    // The folder icon stays put; a control that does nothing is worse than none.
    const { queryByLabelText } = renderWorktreeItem()
    expect(queryByLabelText('Toggle sessions')).not.toBeInTheDocument()
  })

  it('expands and collapses on click', () => {
    const onToggle = vi.fn()
    const { getByLabelText } = renderWorktreeItem(worktree, vi.fn(), {
      onToggleSessionsExpanded: onToggle,
      sessionsExpanded: false
    })

    fireEvent.click(getByLabelText('Toggle sessions'))
    expect(onToggle).toHaveBeenCalledTimes(1)
  })

  it('does not select the worktree just because the toggle was clicked', () => {
    // The toggle sits inside the row; without stopPropagation, opening the
    // sessions would also switch to the worktree.
    const onSelect = vi.fn()
    const { getByLabelText } = renderWorktreeItem(worktree, vi.fn(), {
      onSelect,
      onToggleSessionsExpanded: vi.fn()
    })

    fireEvent.click(getByLabelText('Toggle sessions'))
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('works from the keyboard, since it is not a real button', () => {
    const onToggle = vi.fn()
    const { getByLabelText } = renderWorktreeItem(worktree, vi.fn(), {
      onToggleSessionsExpanded: onToggle
    })
    const toggle = getByLabelText('Toggle sessions')

    fireEvent.keyDown(toggle, { key: 'Enter' })
    fireEvent.keyDown(toggle, { key: ' ' })
    expect(onToggle).toHaveBeenCalledTimes(2)

    fireEvent.keyDown(toggle, { key: 'a' })
    expect(onToggle).toHaveBeenCalledTimes(2)
  })

  it('reports collapsed rather than nothing before anything sets the state', () => {
    const { getByLabelText } = renderWorktreeItem(worktree, vi.fn(), {
      onToggleSessionsExpanded: vi.fn()
    })
    expect(getByLabelText('Toggle sessions')).toHaveAttribute('aria-expanded', 'false')
  })

  it('says whether it is expanded, so a screen reader is not guessing', () => {
    const { getByLabelText, rerender } = renderWorktreeItem(worktree, vi.fn(), {
      onToggleSessionsExpanded: vi.fn(),
      sessionsExpanded: false
    })
    expect(getByLabelText('Toggle sessions')).toHaveAttribute('aria-expanded', 'false')

    rerender(
      <WorktreeItem
        worktree={worktree}
        projectPath={project.path}
        projectName={project.name}
        isActiveWorktree={false}
        sessionCount={0}
        onSelect={vi.fn()}
        onWorktreesChanged={vi.fn()}
        onToggleSessionsExpanded={vi.fn()}
        sessionsExpanded
      />
    )
    expect(getByLabelText('Toggle sessions')).toHaveAttribute('aria-expanded', 'true')
  })
})

describe('WorktreeItem rename', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    // The real contract: the new path and name, or null when it failed.
    mockRenameWorktree.mockResolvedValue({ newPath: '/tmp/p/renamed', name: 'renamed' })
    useAppStore.setState({ config: baseConfig as AppConfig })
  })

  afterEach(() => {
    useAppStore.setState(initialState)
  })

  /** Enter rename mode and hand back the input it focuses. */
  function startRenaming(onWorktreesChanged = vi.fn()) {
    const utils = renderWorktreeItem(worktree, onWorktreesChanged)
    fireEvent.click(utils.getByLabelText('Rename worktree'))
    return { ...utils, input: utils.container.querySelector('input') as HTMLInputElement }
  }

  it('opens on the current name, so a small edit is a small edit', () => {
    const { input } = startRenaming()
    expect(input).toHaveValue('feature-a')
  })

  it('renames on Enter and tells the sidebar to reload', async () => {
    const onWorktreesChanged = vi.fn()
    const { input } = startRenaming(onWorktreesChanged)

    fireEvent.change(input, { target: { value: 'renamed' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => expect(mockRenameWorktree).toHaveBeenCalledWith(worktree.path, 'renamed'))
    // Without this the row keeps showing the old name until something else
    // happens to refresh it.
    await waitFor(() => expect(onWorktreesChanged).toHaveBeenCalled())
  })

  it('trims what was typed rather than creating " renamed"', async () => {
    const { input } = startRenaming()
    fireEvent.change(input, { target: { value: '  renamed  ' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => expect(mockRenameWorktree).toHaveBeenCalledWith(worktree.path, 'renamed'))
  })

  it('says so when the rename was refused', async () => {
    mockRenameWorktree.mockResolvedValue(null)
    const onWorktreesChanged = vi.fn()
    const { input } = startRenaming(onWorktreesChanged)

    fireEvent.change(input, { target: { value: 'renamed' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => expect(mockToastError).toHaveBeenCalledWith('Failed to rename worktree'))
    // Nothing changed, so nothing to reload.
    expect(onWorktreesChanged).not.toHaveBeenCalled()
  })

  it('does nothing at all when the name was not changed', async () => {
    const { input } = startRenaming()
    fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => expect(document.querySelector('input')).toBeNull())
    expect(mockRenameWorktree).not.toHaveBeenCalled()
  })

  it('abandons the rename on Escape', async () => {
    const { input } = startRenaming()
    fireEvent.change(input, { target: { value: 'renamed' } })
    fireEvent.keyDown(input, { key: 'Escape' })

    await waitFor(() => expect(document.querySelector('input')).toBeNull())
    expect(mockRenameWorktree).not.toHaveBeenCalled()
  })

  it('treats a name emptied to nothing as a cancel, not a rename to ""', async () => {
    const { input } = startRenaming()
    fireEvent.change(input, { target: { value: '   ' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => expect(document.querySelector('input')).toBeNull())
    expect(mockRenameWorktree).not.toHaveBeenCalled()
  })

  it('commits when focus leaves, so clicking away is not a silent discard', async () => {
    const { input } = startRenaming()
    fireEvent.change(input, { target: { value: 'renamed' } })
    fireEvent.blur(input)

    await waitFor(() => expect(mockRenameWorktree).toHaveBeenCalledWith(worktree.path, 'renamed'))
  })
})

// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockStore = {
  sidebarProjectSort: 'manual',
  sidebarWorktreeSort: 'name',
  sidebarWorktreeFilter: 'all',
  sidebarViewMode: 'worktrees' as string,
  setSidebarProjectSort: vi.fn(),
  setSidebarWorktreeSort: vi.fn(),
  setSidebarWorktreeFilter: vi.fn(),
  setSidebarViewMode: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockStore) : mockStore
}))

const { ProjectsSectionToolbar } =
  await import('../src/renderer/components/project-sidebar/ProjectsSectionToolbar')

beforeEach(() => {
  vi.clearAllMocks()
  mockStore.sidebarViewMode = 'worktrees'
})

const open = () => {
  render(<ProjectsSectionToolbar />)
  fireEvent.click(screen.getByRole('button', { name: 'Filter & sort' }))
}

describe('the Show menu', () => {
  it('offers Groups > Sessions beside the others', () => {
    open()
    expect(screen.getByText('Projects > Worktrees')).toBeInTheDocument()
    expect(screen.getByText('Groups > Sessions')).toBeInTheDocument()
    expect(screen.getByText('Sessions (Flat)')).toBeInTheDocument()
  })

  it('switches to it', () => {
    open()
    fireEvent.click(screen.getByText('Groups > Sessions'))
    expect(mockStore.setSidebarViewMode).toHaveBeenCalledWith('groups-sessions')
  })

  /**
   * The mode draws no project or worktree rows, so the controls that reorder and
   * filter them have nothing to act on.
   */
  it('hides the filter and both sorts in that mode', () => {
    mockStore.sidebarViewMode = 'groups-sessions'
    open()
    expect(screen.queryByText('Filter')).not.toBeInTheDocument()
    expect(screen.queryByText('Sort projects')).not.toBeInTheDocument()
    expect(screen.queryByText('Sort worktrees')).not.toBeInTheDocument()
  })

  it('still offers them in a project mode', () => {
    open()
    expect(screen.getByText('Filter')).toBeInTheDocument()
    expect(screen.getByText('Sort projects')).toBeInTheDocument()
    expect(screen.getByText('Sort worktrees')).toBeInTheDocument()
  })

  it('marks the mode as non-default once it is chosen', () => {
    mockStore.sidebarViewMode = 'groups-sessions'
    render(<ProjectsSectionToolbar />)
    expect(screen.getByRole('button', { name: 'Filter & sort' })).toBeInTheDocument()
  })
})

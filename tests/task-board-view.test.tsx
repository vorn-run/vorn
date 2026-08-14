// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockState = {
  activeProject: null,
  config: { tasks: [], projects: [] },
  removeTask: vi.fn(),
  addTerminal: vi.fn(),
  startTask: vi.fn(),
  completeTask: vi.fn(),
  cancelTask: vi.fn(),
  reopenTask: vi.fn(),
  updateTask: vi.fn(),
  terminals: new Map(),
  setFocusedTerminal: vi.fn(),
  setSelectedTaskId: vi.fn(),
  setTaskDialogOpen: vi.fn(),
  taskStatusFilter: 'all',
  activeWorkspace: 'personal'
}

vi.mock('../src/renderer/stores', () => {
  const useAppStore = (selector?: (s: unknown) => unknown) =>
    selector ? selector(mockState) : mockState
  useAppStore.getState = () => mockState
  return { useAppStore }
})

vi.mock('../src/renderer/hooks/useWorkspaceProjects', () => ({
  useWorkspaceProjects: () => []
}))

vi.mock('../src/renderer/components/task-board/TaskKanbanBoard', () => ({
  TaskKanbanBoard: () => <div data-testid="kanban" />
}))

vi.mock('../src/renderer/components/task-board/TaskListView', () => ({
  TaskListView: () => <div data-testid="list" />
}))

vi.mock('../src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() }
}))

const { TaskBoardView } = await import('../src/renderer/components/TaskBoardView')

describe('TaskBoardView', () => {
  it('renders the empty state when there are no tasks', () => {
    const { container, getByText } = render(<TaskBoardView />)
    expect(getByText(/No tasks yet/i)).toBeInTheDocument()
    const root = container.firstElementChild as HTMLElement
    // The board is a main view and sits on the app field, the same rung the
    // workflow canvas uses — it was a step higher, which is what made tasks read
    // pale next to workflows.
    expect(root.style.background).toBe('var(--color-surface-base)')
  })
})

// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { TaskKanbanBoard } from '../src/renderer/components/task-board/TaskKanbanBoard'
import type { TaskConfig, TaskStatus } from '../packages/shared/src/types'

// Stub TaskCard to a plain div — we're testing column layout, not card internals
vi.mock('../src/renderer/components/task-board/TaskCard', () => ({
  TaskCard: ({ task }: { task: TaskConfig }) => (
    <div data-testid={`card-${task.id}`}>{task.title}</div>
  )
}))

function makeTask(
  overrides: Partial<TaskConfig> & { status: TaskStatus; order: number }
): TaskConfig {
  return {
    id: `task-${overrides.order}`,
    projectName: 'test',
    title: `Task ${overrides.order}`,
    description: '',
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    ...overrides
  }
}

const noop = () => {}
const noopTask = () => undefined

const defaultProps = {
  onEdit: noop as (t: TaskConfig) => void,
  onDelete: noop,
  onStart: noop as (t: TaskConfig) => void,
  onDrop: noop,
  onOpenSession: noopTask as (t: TaskConfig) => (() => void) | undefined,
  onComplete: noop,
  onCancel: noop,
  onReopen: noop,
  onArchive: noop,
  onUnarchive: noop,
  onReviewDiff: noop,
  isSessionLive: () => false
}

describe('TaskKanbanBoard', () => {
  it('renders all five columns', () => {
    const { container } = render(<TaskKanbanBoard allTasks={[]} {...defaultProps} />)
    const columns = container.querySelectorAll('.group\\/col')
    expect(columns).toHaveLength(5)
  })

  it('columns have min-h-0 so overflow-y-auto can activate', () => {
    const { container } = render(<TaskKanbanBoard allTasks={[]} {...defaultProps} />)
    const columns = container.querySelectorAll('.group\\/col')
    columns.forEach((col) => {
      expect(col).toHaveClass('min-h-0')
    })
  })

  it('cards container has overflow-y-auto for scrolling', () => {
    const tasks = Array.from({ length: 20 }, (_, i) => makeTask({ status: 'todo', order: i }))
    const { container } = render(<TaskKanbanBoard allTasks={tasks} {...defaultProps} />)
    const scrollContainers = container.querySelectorAll('.overflow-y-auto')
    expect(scrollContainers.length).toBeGreaterThanOrEqual(1)
  })

  it('leaves the field showing through a column at rest', () => {
    // A column groups cards; it is not a material sitting on the board. Filling
    // it lit five tall slabs a step above the field and is what made this view
    // read pale beside the workflow canvas, where only the nodes lift.
    const { container } = render(<TaskKanbanBoard allTasks={[]} {...defaultProps} />)
    container.querySelectorAll('.group\\/col').forEach((col) => {
      expect(col.className).not.toMatch(/(^|\s)bg-/)
    })
  })

  it('renders cards inside the correct column', () => {
    const tasks = [
      makeTask({ status: 'todo', order: 0 }),
      makeTask({ status: 'in_progress', order: 1 })
    ]
    const { getByText } = render(<TaskKanbanBoard allTasks={tasks} {...defaultProps} />)
    expect(getByText('Task 0')).toBeInTheDocument()
    expect(getByText('Task 1')).toBeInTheDocument()
  })
})

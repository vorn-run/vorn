// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// AnimatePresence keeps a collapsing section mounted for its exit animation,
// which jsdom never finishes — the same stub status-picker.test.tsx uses.
vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: React.PropsWithChildren<Record<string, unknown>>) => (
      <div {...props}>{children}</div>
    )
  },
  AnimatePresence: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

vi.mock('../src/renderer/components/task-board/TaskCard', () => ({
  TaskCard: ({ task }: { task: { id: string; title: string } }) => (
    <div data-testid={`card-${task.id}`}>{task.title}</div>
  )
}))

import { TaskListView } from '../src/renderer/components/task-board/TaskListView'
import { TASK_STATUS_TEXT, TASK_STATUS_LABEL } from '../src/renderer/lib/task-status'
import type { TaskConfig, TaskStatus } from '../src/shared/types'

const task = (id: string, status: TaskStatus): TaskConfig =>
  ({ id, title: `Task ${id}`, status, projectName: 'Vorn', createdAt: '2026-08-01' }) as TaskConfig

const section = (status: TaskStatus, tasks: TaskConfig[] = []) => ({
  status,
  title: TASK_STATUS_LABEL[status],
  tasks,
  emptyText: `No ${status} tasks`
})

const handlers = {
  onEdit: vi.fn(),
  onDelete: vi.fn(),
  onOpenSession: () => undefined,
  onComplete: vi.fn(),
  onCancel: vi.fn(),
  onReopen: vi.fn(),
  onArchive: vi.fn(),
  onUnarchive: vi.fn(),
  onReviewDiff: vi.fn(),
  isSessionLive: () => false
}

afterEach(cleanup)

describe('TaskListView', () => {
  it('heads each section with its status glyph in the shared tone', () => {
    // The section header and the card beneath it read the same map — they used
    // to reach for it through two different names.
    const { container } = render(
      <TaskListView {...handlers} sections={[section('in_review', [task('a', 'in_review')])]} />
    )
    expect(screen.getByText('In Review')).toBeInTheDocument()
    expect(container.querySelector(`.${TASK_STATUS_TEXT.in_review}`)).toBeInTheDocument()
  })

  it('accents the review section and nothing else', () => {
    // Bronzo marks work handed back to the person; a backlog does not qualify.
    const { container } = render(
      <TaskListView
        {...handlers}
        sections={[
          section('todo', [task('a', 'todo')]),
          section('in_progress', [task('b', 'in_progress')]),
          section('done', [task('c', 'done')])
        ]}
      />
    )
    expect(container.querySelector('[class*="bronzo"]')).toBeNull()
  })

  it('says what an empty section is empty of', () => {
    render(<TaskListView {...handlers} sections={[section('done')]} />)
    expect(screen.getByText('No done tasks')).toBeInTheDocument()
  })

  it('collapses a section without losing its heading', () => {
    render(<TaskListView {...handlers} sections={[section('todo', [task('a', 'todo')])]} />)
    expect(screen.getByTestId('card-a')).toBeInTheDocument()

    fireEvent.click(screen.getByText('Todo'))
    expect(screen.queryByTestId('card-a')).not.toBeInTheDocument()
    expect(screen.getByText('Todo')).toBeInTheDocument()
  })
})

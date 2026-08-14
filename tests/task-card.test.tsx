// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.hoisted(() => {
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
})

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (s: unknown) => unknown) => {
    const state = { connections: [], config: { connections: [] } }
    return selector ? selector(state) : state
  }
}))
vi.mock('../src/renderer/components/task-board/KanbanCardMenu', () => ({
  KanbanCardMenu: () => <div data-testid="kanban-menu" />
}))

import { TaskCard } from '../src/renderer/components/task-board/TaskCard'
import { TASK_STATUS_TEXT, TASK_LIVE_DOT } from '../src/renderer/lib/task-status'
import type { TaskConfig } from '../src/shared/types'

const task = (over: Partial<TaskConfig> = {}): TaskConfig =>
  ({
    id: 'abcd1234',
    title: 'Wire up the run trace pane',
    status: 'todo',
    projectName: 'Vorn',
    createdAt: '2026-08-01T10:00:00Z',
    ...over
  }) as TaskConfig

const noop = (): void => {}
const base = { onEdit: noop, onDelete: noop }

afterEach(cleanup)

const VARIANTS = ['default', 'kanban'] as const

describe('TaskCard', () => {
  for (const variant of VARIANTS) {
    it(`shows the task's status through the shared map (${variant})`, () => {
      // Status is a glyph on this surface, and its colour used to come from a
      // map that disagreed with the three others: every in-progress task drew a
      // yellow icon while the rest of the app called that state blue.
      const { container } = render(
        <TaskCard {...base} task={task({ status: 'in_progress' })} variant={variant} />
      )
      expect(container.querySelector(`.${TASK_STATUS_TEXT.in_progress}`)).toBeInTheDocument()
    })

    it(`spends the accent only on a task waiting for review (${variant})`, () => {
      // in_review is an agent handing work back — the same relationship as a
      // waiting session or an open gate, and the only one bronzo may mark here.
      const review = render(
        <TaskCard {...base} task={task({ status: 'in_review' })} variant={variant} />
      )
      expect(review.container.querySelector('[class*="bronzo"]')).toBeInTheDocument()
      cleanup()

      for (const status of ['todo', 'in_progress', 'done', 'cancelled'] as const) {
        const { container } = render(
          <TaskCard {...base} task={task({ status })} variant={variant} />
        )
        expect(container.querySelector('[class*="bronzo"]')).toBeNull()
        cleanup()
      }
    })

    it(`strikes through and dims a cancelled task (${variant})`, () => {
      const { container } = render(
        <TaskCard {...base} task={task({ status: 'cancelled' })} variant={variant} />
      )
      expect(container.querySelector('.line-through')).toBeInTheDocument()
      expect(container.querySelector('.opacity-60')).toBeInTheDocument()
    })

    it(`names the task and its short id (${variant})`, () => {
      render(<TaskCard {...base} task={task()} variant={variant} />)
      expect(screen.getByText('Wire up the run trace pane')).toBeInTheDocument()
      // VOR- from the project name, ABCD from the id.
      expect(screen.getByText(/VOR-ABCD/)).toBeInTheDocument()
    })
  }

  it('lifts a kanban card off the field with a ladder rung, not a wash', () => {
    // A translucent white fill greys toward the white it is made of, so these
    // cards read pale while workflow nodes at the same lightness read as part
    // of the app. Same relationship, so the same treatment: an opaque surface
    // token plus a hairline.
    const { container } = render(<TaskCard {...base} task={task()} variant="kanban" />)
    const card = container.firstElementChild as HTMLElement
    expect(card.className).toContain('bg-surface-raised')
    expect(card.className).not.toMatch(/(^|\s)bg-white\//)
  })

  it('marks a live session with the one moving thing on the board', () => {
    // Motion is a report of work in progress; nothing else on this surface
    // moves. The dot only appears once an agent is actually assigned.
    const { container } = render(
      <TaskCard
        {...base}
        task={task({ status: 'in_progress', assignedAgent: 'claude' })}
        sessionIsLive
        variant="kanban"
      />
    )
    const dot = container.querySelector(`.${TASK_LIVE_DOT.split(' ')[0]}`)
    expect(dot).toBeInTheDocument()
    expect(dot?.className).toContain('animate-pulse')
  })

  it('selects rather than edits when a row is clicked', () => {
    // Clicking a task opens the detail panel; "edit" and "select" are the same
    // gesture, and nothing on the card itself draws a selected state.
    const onSelect = vi.fn()
    render(<TaskCard {...base} task={task()} onSelect={onSelect} />)
    fireEvent.click(screen.getByText('Wire up the run trace pane'))
    expect(onSelect).toHaveBeenCalled()
  })
})

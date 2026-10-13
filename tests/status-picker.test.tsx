// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('react-dom', async () => {
  const actual = await vi.importActual<typeof import('react-dom')>('react-dom')
  return { ...actual, createPortal: (node: React.ReactNode) => node }
})

vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: React.PropsWithChildren<Record<string, unknown>>) => (
      <div {...props}>{children}</div>
    )
  },
  AnimatePresence: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

const completeTask = vi.fn()
const cancelTask = vi.fn()
const reopenTask = vi.fn()
const reviewTask = vi.fn()
const updateTask = vi.fn()

const mockState = { completeTask, cancelTask, reopenTask, reviewTask, updateTask }

vi.mock('../packages/web/src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockState) : mockState
}))

vi.mock('../packages/web/src/renderer/components/Toast', () => ({
  toast: { success: vi.fn(), info: vi.fn(), error: vi.fn() }
}))

const { StatusPicker } = await import('../packages/web/src/renderer/components/StatusPicker')

describe('StatusPicker — todo → in_progress routing', () => {
  beforeEach(() => {
    completeTask.mockReset()
    cancelTask.mockReset()
    reopenTask.mockReset()
    reviewTask.mockReset()
    updateTask.mockReset()
  })

  it('calls updateTask with in_progress when selecting In Progress from todo', () => {
    const { getByText, getByRole } = render(<StatusPicker taskId="t1" currentStatus="todo" />)
    fireEvent.click(getByRole('button', { name: /Todo/ }))
    fireEvent.click(getByText('In Progress'))
    expect(updateTask).toHaveBeenCalledWith('t1', { status: 'in_progress' })
    expect(reopenTask).not.toHaveBeenCalled()
  })

  it('disables In Progress when currentStatus is done (store mode)', () => {
    const { getByText, getByRole } = render(<StatusPicker taskId="t1" currentStatus="done" />)
    fireEvent.click(getByRole('button', { name: /Done/ }))
    const inProgressItem = getByText('In Progress').closest('button')!
    expect(inProgressItem).toBeDisabled()
    fireEvent.click(inProgressItem)
    expect(updateTask).not.toHaveBeenCalled()
  })

  it('disables In Progress when currentStatus is in_review', () => {
    const { getByText, getByRole } = render(<StatusPicker taskId="t1" currentStatus="in_review" />)
    fireEvent.click(getByRole('button', { name: /In Review/ }))
    const inProgressItem = getByText('In Progress').closest('button')!
    expect(inProgressItem).toBeDisabled()
  })

  it('does NOT disable In Progress in controlled mode (onChange provided)', () => {
    const onChange = vi.fn()
    const { getByText, getByRole } = render(
      <StatusPicker currentStatus="done" onChange={onChange} />
    )
    fireEvent.click(getByRole('button', { name: /Done/ }))
    const inProgressItem = getByText('In Progress').closest('button')!
    expect(inProgressItem).not.toBeDisabled()
    fireEvent.click(inProgressItem)
    expect(onChange).toHaveBeenCalledWith('in_progress')
  })

  it('routes other statuses through their dedicated store methods', () => {
    const { getByText, getByRole, rerender } = render(
      <StatusPicker taskId="t1" currentStatus="todo" />
    )
    fireEvent.click(getByRole('button', { name: /Todo/ }))
    fireEvent.click(getByText('Done'))
    expect(completeTask).toHaveBeenCalledWith('t1')

    rerender(<StatusPicker taskId="t1" currentStatus="done" />)
    fireEvent.click(getByRole('button', { name: /Done/ }))
    fireEvent.click(getByText('Todo'))
    expect(reopenTask).toHaveBeenCalledWith('t1')
  })
})

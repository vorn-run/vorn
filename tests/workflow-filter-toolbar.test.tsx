// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const mockState = {
  sidebarWorkflowFilter: 'all' as 'all' | 'manual' | 'scheduled',
  setSidebarWorkflowFilter: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    return selector ? selector(mockState) : mockState
  }
}))

const { WorkflowFilterToolbar } =
  await import('../src/renderer/components/project-sidebar/WorkflowFilterToolbar')

beforeEach(() => {
  mockState.sidebarWorkflowFilter = 'all'
  mockState.setSidebarWorkflowFilter.mockReset()
})

describe('WorkflowFilterToolbar', () => {
  it('renders the filter button', () => {
    const { container } = render(<WorkflowFilterToolbar />)
    expect(container.querySelector('button')).toBeInTheDocument()
  })

  it('shows the indicator dot when a non-default filter is active', () => {
    mockState.sidebarWorkflowFilter = 'manual'
    const { container } = render(<WorkflowFilterToolbar />)
    expect(container.querySelector('.bg-ink-secondary')).toBeInTheDocument()
  })

  it('does not show the indicator dot for the default "all" filter', () => {
    const { container } = render(<WorkflowFilterToolbar />)
    expect(container.querySelector('.bg-ink-secondary')).not.toBeInTheDocument()
  })

  it('opens a dropdown with All / Manual / Scheduled options when clicked', () => {
    render(<WorkflowFilterToolbar />)
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByText('All')).toBeInTheDocument()
    expect(screen.getByText('Manual')).toBeInTheDocument()
    expect(screen.getByText('Scheduled')).toBeInTheDocument()
  })

  it('calls setSidebarWorkflowFilter when an option is selected', () => {
    render(<WorkflowFilterToolbar />)
    fireEvent.click(screen.getByRole('button'))
    fireEvent.click(screen.getByText('Manual'))
    expect(mockState.setSidebarWorkflowFilter).toHaveBeenCalledWith('manual')
  })
})

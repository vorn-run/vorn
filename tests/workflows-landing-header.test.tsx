// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ReactNode } from 'react'
import type { RunBucket } from '../packages/web/src/renderer/stores/types'

vi.mock('../packages/web/src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: { children: ReactNode }) => <>{children}</>
}))

const mockState = {
  workflowsLandingTab: 'runs' as 'runs' | 'review',
  setWorkflowsLandingTab: vi.fn(),
  workflowsRunFilter: 'all' as RunBucket,
  setWorkflowsRunFilter: vi.fn(),
  workflowsRunsInflight: 0,
  bumpWorkflowsRunsReload: vi.fn()
}

vi.mock('../packages/web/src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    return selector ? selector(mockState) : mockState
  }
}))

const waitingMock = vi.fn(() => [] as unknown[])
vi.mock('../packages/web/src/renderer/hooks/useWaitingApprovals', () => ({
  useWaitingApprovals: () => waitingMock()
}))

const { WorkflowsLandingHeader } =
  await import('../packages/web/src/renderer/components/workflow-runs/WorkflowsLandingHeader')

beforeEach(() => {
  mockState.workflowsLandingTab = 'runs'
  mockState.workflowsRunFilter = 'all'
  mockState.workflowsRunsInflight = 0
  mockState.setWorkflowsLandingTab.mockReset()
  mockState.setWorkflowsRunFilter.mockReset()
  mockState.bumpWorkflowsRunsReload.mockReset()
  waitingMock.mockReset()
  waitingMock.mockReturnValue([])
})

describe('WorkflowsLandingHeader', () => {
  it('renders both tabs and the refresh control', () => {
    render(<WorkflowsLandingHeader />)
    expect(screen.getByRole('button', { name: 'All runs' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Needs review/ })).toBeInTheDocument()
    expect(screen.getByLabelText('Refresh')).toBeInTheDocument()
  })

  it('shows the filter toolbar only on the All runs tab', () => {
    const { rerender } = render(<WorkflowsLandingHeader />)
    expect(screen.getByLabelText('Filter runs')).toBeInTheDocument()

    mockState.workflowsLandingTab = 'review'
    rerender(<WorkflowsLandingHeader />)
    expect(screen.queryByLabelText('Filter runs')).not.toBeInTheDocument()
  })

  it('switches tabs via the store actions', () => {
    render(<WorkflowsLandingHeader />)
    fireEvent.click(screen.getByRole('button', { name: /Needs review/ }))
    expect(mockState.setWorkflowsLandingTab).toHaveBeenCalledWith('review')

    mockState.workflowsLandingTab = 'review'
    fireEvent.click(screen.getByRole('button', { name: 'All runs' }))
    expect(mockState.setWorkflowsLandingTab).toHaveBeenCalledWith('runs')
  })

  it('triggers the store reload on refresh click', () => {
    render(<WorkflowsLandingHeader />)
    fireEvent.click(screen.getByLabelText('Refresh'))
    expect(mockState.bumpWorkflowsRunsReload).toHaveBeenCalled()
  })

  it('spins the refresh icon while loading is true', () => {
    mockState.workflowsRunsInflight = 2
    const { container } = render(<WorkflowsLandingHeader />)
    expect(container.querySelector('.animate-spin')).toBeInTheDocument()
  })

  it('renders the waiting count next to Needs review when approvals are pending', () => {
    waitingMock.mockReturnValue([{}, {}, {}])
    render(<WorkflowsLandingHeader />)
    expect(screen.getByText('3')).toBeInTheDocument()
  })

  it('omits the waiting count when no approvals are pending', () => {
    render(<WorkflowsLandingHeader />)
    const reviewButton = screen.getByRole('button', { name: /Needs review/ })
    expect(reviewButton.querySelector('.text-amber-400')).not.toBeInTheDocument()
  })
})

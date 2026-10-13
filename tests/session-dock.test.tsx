// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WaitingApproval } from '../packages/web/src/renderer/hooks/useWaitingApprovals'

let minimizedIds: string[] = []
let waitingApprovals: WaitingApproval[] = []
let collapsed = false
const toggleCollapsed = vi.fn()

vi.mock('../packages/web/src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    const state = {
      sessionDockCollapsed: collapsed,
      toggleSessionDockCollapsed: toggleCollapsed
    }
    return selector ? selector(state) : state
  }
}))

vi.mock('../packages/web/src/renderer/hooks/useVisibleTerminals', () => ({
  useVisibleTerminals: () => ({ orderedIds: [], minimizedIds })
}))
vi.mock('../packages/web/src/renderer/hooks/useWaitingApprovals', () => ({
  useWaitingApprovals: () => waitingApprovals
}))

vi.mock('../packages/web/src/renderer/components/MinimizedPill', () => ({
  MinimizedPill: ({ terminalId }: { terminalId: string }) => (
    <div data-testid="minimized-pill" data-id={terminalId}>
      {terminalId}
    </div>
  )
}))
vi.mock('../packages/web/src/renderer/components/WaitingApprovalPill', () => ({
  WaitingApprovalPill: ({
    execution,
    nodeState
  }: {
    execution: { workflowId: string }
    nodeState: { nodeId: string }
  }) => (
    <div data-testid="waiting-pill">
      {execution.workflowId}:{nodeState.nodeId}
    </div>
  )
}))

import { SessionDock } from '../packages/web/src/renderer/components/SessionDock'

function waiting(workflowId = 'wf-1', nodeId = 'n1'): WaitingApproval {
  return {
    execution: {
      workflowId,
      startedAt: '2026-04-20T10:00:00Z',
      status: 'running',
      nodeStates: [{ nodeId, status: 'waiting' }]
    } as WaitingApproval['execution'],
    nodeState: { nodeId, status: 'waiting' } as WaitingApproval['nodeState']
  }
}

beforeEach(() => {
  minimizedIds = []
  waitingApprovals = []
  collapsed = false
  toggleCollapsed.mockReset()
})

describe('SessionDock', () => {
  it('returns null when there are no items', () => {
    const { container } = render(<SessionDock includeMinimized={true} />)
    expect(container.firstChild).toBeNull()
  })

  it('renders waiting → minimized pills inline up to MAX_INLINE (4)', () => {
    waitingApprovals = [waiting('wf', 'n1')]
    minimizedIds = ['m1', 'm2']
    const { getAllByTestId, queryByText } = render(<SessionDock includeMinimized={true} />)
    expect(getAllByTestId('waiting-pill')).toHaveLength(1)
    expect(getAllByTestId('minimized-pill')).toHaveLength(2)
    expect(queryByText(/^\+\d+$/)).toBeNull()
  })

  it('caps inline at 4 and overflows the rest into a +N badge', () => {
    minimizedIds = ['a', 'b', 'c', 'd', 'e', 'f']
    const { getAllByTestId, getByText } = render(<SessionDock includeMinimized={true} />)
    expect(getAllByTestId('minimized-pill')).toHaveLength(4)
    expect(getByText('+2')).toBeInTheDocument()
  })

  it('clicking +N opens a popover with the overflow items', () => {
    minimizedIds = ['a', 'b', 'c', 'd', 'e', 'f', 'g']
    const { getByText, getAllByTestId } = render(<SessionDock includeMinimized={true} />)
    fireEvent.click(getByText('+3'))
    // 4 inline + 3 overflow = 7 minimized pills total visible
    expect(getAllByTestId('minimized-pill')).toHaveLength(7)
  })

  it('omits minimized items and only shows waiting when includeMinimized=false (tab mode)', () => {
    minimizedIds = ['a', 'b', 'c']
    waitingApprovals = [waiting()]
    const { queryAllByTestId } = render(<SessionDock includeMinimized={false} />)
    expect(queryAllByTestId('minimized-pill')).toHaveLength(0)
    expect(queryAllByTestId('waiting-pill')).toHaveLength(1)
  })

  it('renders a divider between consecutive groups in the inline strip', () => {
    waitingApprovals = [waiting()]
    minimizedIds = ['m1']
    const { container } = render(<SessionDock includeMinimized={true} />)
    // 2 groups = 1 separator
    const dividers = container.querySelectorAll('div[aria-hidden="true"]')
    expect(dividers.length).toBe(1)
  })

  it('does not render a divider when only one group is present', () => {
    minimizedIds = ['a', 'b']
    const { container } = render(<SessionDock includeMinimized={true} />)
    const dividers = container.querySelectorAll('div[aria-hidden="true"]')
    expect(dividers.length).toBe(0)
  })

  it('clicking the trailing chevron toggles the collapsed flag', () => {
    minimizedIds = ['a', 'b']
    const { getByLabelText } = render(<SessionDock includeMinimized={true} />)
    fireEvent.click(getByLabelText('Collapse session dock'))
    expect(toggleCollapsed).toHaveBeenCalledTimes(1)
  })

  it('collapsed badge tooltip describes the breakdown ("2 minimized")', () => {
    collapsed = true
    minimizedIds = ['a', 'b']
    const { getByLabelText } = render(<SessionDock includeMinimized={true} />)
    const badge = getByLabelText('2 minimized')
    expect(badge).toBeInTheDocument()
    expect(badge.textContent).toContain('2')
  })

  it('collapsed badge tooltip joins multiple groups with separator', () => {
    collapsed = true
    minimizedIds = ['a', 'b']
    waitingApprovals = [waiting()]
    const { getByLabelText } = render(<SessionDock includeMinimized={true} />)
    const badge = getByLabelText('1 waiting approval · 2 minimized')
    expect(badge).toBeInTheDocument()
  })

  it('clicking the badge opens a popover with every dock item', () => {
    collapsed = true
    minimizedIds = ['a', 'b']
    waitingApprovals = [waiting()]
    const { getByLabelText, getAllByTestId } = render(<SessionDock includeMinimized={true} />)
    fireEvent.click(getByLabelText('1 waiting approval · 2 minimized'))
    expect(getAllByTestId('minimized-pill')).toHaveLength(2)
    expect(getAllByTestId('waiting-pill')).toHaveLength(1)
  })

  it('Expand button inside the badge popover toggles the dock back inline', () => {
    collapsed = true
    minimizedIds = ['a']
    const { getByLabelText, getByTitle } = render(<SessionDock includeMinimized={true} />)
    fireEvent.click(getByLabelText('1 minimized'))
    fireEvent.click(getByTitle('Expand dock'))
    expect(toggleCollapsed).toHaveBeenCalledTimes(1)
  })

  it('popover hides section labels when only one group is present', () => {
    collapsed = true
    minimizedIds = ['a', 'b']
    const { getByLabelText, queryByText } = render(<SessionDock includeMinimized={true} />)
    fireEvent.click(getByLabelText('2 minimized'))
    // No "MINIMIZED" heading inside the popover when it's the only group
    expect(queryByText(/^minimized$/i)).toBeNull()
  })

  it('popover shows section labels when multiple groups are present', () => {
    collapsed = true
    minimizedIds = ['a', 'b']
    waitingApprovals = [waiting()]
    const { getByLabelText, getByText } = render(<SessionDock includeMinimized={true} />)
    fireEvent.click(getByLabelText('1 waiting approval · 2 minimized'))
    expect(getByText(/^minimized$/i)).toBeInTheDocument()
    expect(getByText(/^waiting approval$/i)).toBeInTheDocument()
  })
})

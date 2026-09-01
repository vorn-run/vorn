// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { WORKFLOW_STATUS_DOT } from '../src/renderer/lib/workflow-status'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowDefinition } from '../src/shared/types'

const mockStore = {
  setEditingWorkflowId: vi.fn(),
  setWorkflowEditorOpen: vi.fn(),
  workflowExecutions: new Map<string, unknown>(),
  editingWorkflowId: null as string | null
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    return selector ? selector(mockStore) : mockStore
  }
}))

const execute = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow: (...args: unknown[]) => execute(...args)
}))

const { WorkflowItem } = await import('../src/renderer/components/project-sidebar/WorkflowItem')

function makeManual(): WorkflowDefinition {
  return {
    id: 'w1',
    name: 'My Workflow',
    icon: 'Zap',
    iconColor: '#ffffff',
    nodes: [
      {
        id: 't',
        type: 'trigger',
        label: 'T',
        config: { triggerType: 'manual' },
        position: { x: 0, y: 0 }
      }
    ],
    edges: [],
    enabled: true
  }
}

function makeScheduled(enabled = true): WorkflowDefinition {
  return {
    ...makeManual(),
    id: 'w2',
    nodes: [
      {
        id: 't',
        type: 'trigger',
        label: 'T',
        config: { triggerType: 'recurring', cron: '* * * * *' },
        position: { x: 0, y: 0 }
      }
    ],
    enabled
  }
}

beforeEach(() => {
  mockStore.setEditingWorkflowId.mockReset()
  mockStore.setWorkflowEditorOpen.mockReset()
  execute.mockReset()
})

describe('WorkflowItem', () => {
  it('renders the workflow name', () => {
    render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={vi.fn()}
      />
    )
    expect(screen.getByText('My Workflow')).toBeInTheDocument()
  })

  it('opens the editor when the row is clicked', () => {
    render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={vi.fn()}
      />
    )
    fireEvent.click(screen.getByText('My Workflow'))
    expect(mockStore.setEditingWorkflowId).toHaveBeenCalledWith('w1')
    expect(mockStore.setWorkflowEditorOpen).toHaveBeenCalledWith(true)
  })

  it('executes the workflow when the Run button is clicked', () => {
    render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={vi.fn()}
      />
    )
    const runButton = screen.getByRole('button', { name: /Run workflow/ })
    fireEvent.click(runButton)
    expect(execute).toHaveBeenCalledTimes(1)
  })

  it('calls onContextMenu on right-click', () => {
    const onContextMenu = vi.fn()
    render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={onContextMenu}
      />
    )
    fireEvent.contextMenu(screen.getByText('My Workflow'))
    expect(onContextMenu).toHaveBeenCalledWith(expect.anything(), 'w1')
  })

  it('calls onContextMenu when the More button is clicked', () => {
    const onContextMenu = vi.fn()
    render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={onContextMenu}
      />
    )
    const moreButton = screen.getByRole('button', { name: /More options/ })
    fireEvent.click(moreButton)
    expect(onContextMenu).toHaveBeenCalledWith(expect.anything(), 'w1')
  })

  it('renders a blue status dot for scheduled + enabled workflows', () => {
    const { container } = render(
      <WorkflowItem
        workflow={makeScheduled(true)}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={vi.fn()}
      />
    )
    expect(container.querySelector('.bg-ink-secondary')).toBeInTheDocument()
  })

  it('keeps the dot visible on a dimmed, disabled row', () => {
    const { container } = render(
      <WorkflowItem
        workflow={makeScheduled(false)}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={vi.fn()}
      />
    )
    // The row is at opacity-40; a ghost dot composited away to nothing there.
    expect(container.querySelector('.bg-ink-faint')).toBeInTheDocument()
    expect(container.querySelector('.opacity-40')).toBeInTheDocument()
  })

  it('renders a red dot when lastRunStatus is error', () => {
    const wf = { ...makeManual(), lastRunStatus: 'error' as const }
    const { container } = render(
      <WorkflowItem workflow={wf} isCollapsed={false} iconSize={14} onContextMenu={vi.fn()} />
    )
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.error}`)).toBeInTheDocument()
  })

  it('hides the name and action buttons when collapsed', () => {
    render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={true}
        iconSize={22}
        onContextMenu={vi.fn()}
      />
    )
    expect(screen.queryByText('My Workflow')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Run workflow/ })).not.toBeInTheDocument()
  })

  it('renders a selected indicator bar and white text when the item is the editing target', () => {
    mockStore.editingWorkflowId = 'w1'
    const { container } = render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={vi.fn()}
      />
    )
    const row = container.querySelector('.group\\/wf') as HTMLElement
    expect(row.className).toContain('text-white')
    expect(container.querySelector('span.absolute.left-0')).toBeInTheDocument()
    mockStore.editingWorkflowId = null
  })

  it('omits the selected indicator bar when collapsed even if selected', () => {
    mockStore.editingWorkflowId = 'w1'
    const { container } = render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={true}
        iconSize={22}
        onContextMenu={vi.fn()}
      />
    )
    expect(container.querySelector('span.absolute.left-0')).not.toBeInTheDocument()
    mockStore.editingWorkflowId = null
  })
})

describe('a gate waiting for approval', () => {
  beforeEach(() => {
    mockStore.workflowExecutions = new Map<string, unknown>()
  })

  function runWith(runId: string, workflowId: string, statuses: string[]): [string, unknown] {
    return [
      runId,
      { runId, workflowId, nodeStates: statuses.map((status, i) => ({ nodeId: `n${i}`, status })) }
    ]
  }

  it('flags the workflow amber while any run waits', () => {
    mockStore.workflowExecutions = new Map([runWith('r1', 'w1', ['waiting'])])
    const { container } = render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={vi.fn()}
      />
    )
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.waiting}`)).toBeInTheDocument()
  })

  it('finds a gate in an older run, not only the newest', () => {
    // Runs of one workflow now proceed in parallel, so the run needing
    // attention is not necessarily the most recent one.
    mockStore.workflowExecutions = new Map([
      runWith('older', 'w1', ['waiting']),
      runWith('newer', 'w1', ['running'])
    ])
    const { container } = render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={vi.fn()}
      />
    )
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.waiting}`)).toBeInTheDocument()
  })

  it('ignores a gate belonging to a different workflow', () => {
    mockStore.workflowExecutions = new Map([runWith('other', 'w-other', ['waiting'])])
    const { container } = render(
      <WorkflowItem
        workflow={makeManual()}
        isCollapsed={false}
        iconSize={14}
        onContextMenu={vi.fn()}
      />
    )
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.waiting}`)).toBeNull()
  })
})

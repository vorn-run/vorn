// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowExecution } from '../src/shared/types'

vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...props }: React.PropsWithChildren<Record<string, unknown>>) => (
      <div {...props}>{children}</div>
    )
  },
  AnimatePresence: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

vi.mock('react-dom', async () => {
  const actual = await vi.importActual<typeof import('react-dom')>('react-dom')
  return { ...actual, createPortal: (node: React.ReactNode) => node }
})

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

vi.mock('../src/renderer/components/SidebarToggleButton', () => ({
  SidebarToggleButton: () => <div />
}))
vi.mock('../src/renderer/components/MainViewPills', () => ({ MainViewPills: () => <div /> }))
vi.mock('../src/renderer/components/WindowControls', () => ({ WindowControls: () => <div /> }))

vi.mock('../src/renderer/components/workflow-editor/WorkflowCanvas', () => ({
  WorkflowCanvas: () => <div data-testid="canvas" />
}))
vi.mock('../src/renderer/components/workflow-editor/panels/NodeConfigPanel', () => ({
  NodeConfigPanel: () => <div data-testid="node-config" />
}))
vi.mock('../src/renderer/components/workflow-editor/panels/StepLibrary', () => ({
  StepLibrary: () => <div data-testid="step-library" />
}))
vi.mock('../src/renderer/components/workflow-editor/panels/WorkflowPropertiesPanel', () => ({
  WorkflowPropertiesPanel: () => <div data-testid="properties-panel" />
}))

const captured = vi.hoisted(() => ({
  historyProps: null as Record<string, unknown> | null
}))
vi.mock('../src/renderer/components/workflow-editor/panels/RunHistoryPanel', () => ({
  RunHistoryPanel: (props: Record<string, unknown>) => {
    captured.historyProps = props
    return <div data-testid="run-history" />
  }
}))

const toastFn = vi.hoisted(() => {
  const fn = vi.fn() as ReturnType<typeof vi.fn> & {
    success: ReturnType<typeof vi.fn>
    error: ReturnType<typeof vi.fn>
    dismiss: ReturnType<typeof vi.fn>
  }
  fn.success = vi.fn()
  fn.error = vi.fn()
  fn.dismiss = vi.fn()
  return fn
})
vi.mock('../src/renderer/components/Toast', () => ({ toast: toastFn }))

const executeWorkflow = vi.hoisted(() => vi.fn().mockResolvedValue(undefined))
const retryRunFromFailure = vi.hoisted(() => vi.fn().mockResolvedValue(undefined))
const stopWorkflowRun = vi.hoisted(() => vi.fn().mockResolvedValue(undefined))
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow,
  retryRunFromFailure,
  rerunWorkflowRun: vi.fn().mockResolvedValue(undefined),
  stopWorkflowRun,
  buildStepOutputsMap: vi.fn(() => ({}))
}))

const workflow = {
  id: 'wf-x',
  name: 'Bench',
  icon: 'Workflow',
  iconColor: '#3b82f6',
  enabled: true,
  workspaceId: 'personal',
  nodes: [
    {
      id: 'trigger',
      type: 'trigger',
      label: 'Manual',
      position: { x: 0, y: 0 },
      config: { triggerType: 'manual' }
    },
    {
      id: 'agent',
      type: 'launchAgent',
      label: 'Do the work',
      slug: 'do_the_work',
      position: { x: 0, y: 140 },
      config: { agentType: 'claude', projectName: '', projectPath: '', headless: true }
    }
  ],
  edges: [{ id: 'e1', source: 'trigger', target: 'agent' }]
}

const mockState = {
  isWorkflowEditorOpen: true,
  isSidebarOpen: true,
  editingWorkflowId: 'wf-x' as string | null,
  setWorkflowEditorOpen: vi.fn(),
  setEditingWorkflowId: vi.fn(),
  addWorkflow: vi.fn(),
  updateWorkflow: vi.fn(),
  removeWorkflow: vi.fn(),
  config: { workflows: [workflow], tasks: [], projects: [], defaults: {} },
  setPendingWorkflowRun: vi.fn(),
  addTerminal: vi.fn(),
  setFocusedTerminal: vi.fn(),
  setSelectedTaskId: vi.fn(),
  activeWorkspace: 'personal',
  workflowExecutions: new Map<string, WorkflowExecution>()
}

vi.mock('../src/renderer/stores', () => {
  const useAppStore = (selector?: (s: unknown) => unknown) =>
    selector ? selector(mockState) : mockState
  useAppStore.getState = () => mockState
  return { useAppStore }
})
;(global as unknown as { window: object }).window = {
  addEventListener: vi.fn(),
  removeEventListener: vi.fn(),
  api: {
    listWorkflowRuns: vi.fn().mockResolvedValue([]),
    listConnectionActions: vi.fn().mockResolvedValue([]),
    createTerminal: vi.fn(),
    isWindowMaximized: vi.fn().mockResolvedValue(false),
    onWindowMaximizedChange: vi.fn(() => () => {}),
    windowMinimize: vi.fn(),
    windowMaximize: vi.fn(),
    windowClose: vi.fn()
  }
}

const { WorkflowEditor } = await import('../src/renderer/components/workflow-editor/WorkflowEditor')

const runningExec = (): WorkflowExecution =>
  ({
    runId: 'r1',
    workflowId: 'wf-x',
    startedAt: new Date().toISOString(),
    status: 'running',
    nodeStates: [
      { nodeId: 'trigger', status: 'success' },
      { nodeId: 'agent', status: 'running' }
    ]
  }) as WorkflowExecution

beforeEach(() => {
  mockState.workflowExecutions.clear()
  captured.historyProps = null
  toastFn.mockClear()
  toastFn.success.mockClear()
  retryRunFromFailure.mockClear()
  executeWorkflow.mockClear()
  stopWorkflowRun.mockClear()
})

describe('the first second of a run', () => {
  it('answers the click before the server does', () => {
    const { getByLabelText, getByTestId, queryByLabelText } = render(<WorkflowEditor inline />)
    expect(queryByLabelText('Stop run')).toBeNull()

    fireEvent.click(getByLabelText('Run workflow'))

    expect(getByLabelText('Stop run')).toBeInTheDocument()
    expect(getByTestId('run-history')).toBeInTheDocument()
  })

  it('pins the panel to the run it launched and stops the real run', () => {
    const { getByLabelText, rerender } = render(<WorkflowEditor inline />)
    fireEvent.click(getByLabelText('Run workflow'))

    act(() => {
      mockState.workflowExecutions.set('r1', runningExec())
    })
    rerender(<WorkflowEditor inline />)

    expect(captured.historyProps?.followRunId).toBe('r1')
    fireEvent.click(getByLabelText('Stop run'))
    expect(stopWorkflowRun).toHaveBeenCalledWith('r1')
  })

  it('never lets a background run hijack the panel', () => {
    const { queryByTestId, rerender } = render(<WorkflowEditor inline />)
    act(() => {
      mockState.workflowExecutions.set('r1', runningExec())
    })
    rerender(<WorkflowEditor inline />)

    expect(queryByTestId('run-history')).toBeNull()
    expect(captured.historyProps).toBeNull()
  })
})

describe('completion toasts', () => {
  function launchAndAdopt() {
    const utils = render(<WorkflowEditor inline />)
    fireEvent.click(utils.getByLabelText('Run workflow'))
    act(() => {
      mockState.workflowExecutions.set('r1', runningExec())
    })
    utils.rerender(<WorkflowEditor inline />)
    return utils
  }

  it('toasts a quiet success with steps and duration', () => {
    const utils = launchAndAdopt()
    act(() => {
      const done = runningExec()
      done.status = 'success'
      done.completedAt = new Date().toISOString()
      done.nodeStates[1].status = 'success'
      mockState.workflowExecutions.set('r1', done)
    })
    utils.rerender(<WorkflowEditor inline />)

    expect(toastFn.success).toHaveBeenCalledWith(expect.stringMatching(/1 step in/))
  })

  it('keeps a failure up, names the step, and retries from it', () => {
    const utils = launchAndAdopt()
    act(() => {
      const failed = runningExec()
      failed.status = 'error'
      failed.nodeStates[1] = { nodeId: 'agent', status: 'error', error: 'Exit code 1' }
      mockState.workflowExecutions.set('r1', failed)
    })
    utils.rerender(<WorkflowEditor inline />)

    expect(toastFn).toHaveBeenCalledWith(
      expect.stringContaining('Do the work'),
      'error',
      expect.objectContaining({ duration: Number.POSITIVE_INFINITY })
    )
    const opts = toastFn.mock.lastCall![2] as {
      actions: { label: string; onClick: (id: string) => void }[]
    }
    expect(opts.actions[0].label).toBe('Retry')
    opts.actions[0].onClick('toast-1')
    expect(toastFn.dismiss).toHaveBeenCalledWith('toast-1')
    expect(retryRunFromFailure).toHaveBeenCalled()
  })
})

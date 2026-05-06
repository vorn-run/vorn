// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

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

vi.mock('lucide-react', () => ({
  ChevronDown: (p: Record<string, unknown>) => <svg data-testid="chev-down" {...p} />,
  ChevronRight: (p: Record<string, unknown>) => <svg data-testid="chev-right" {...p} />,
  Maximize2: (p: Record<string, unknown>) => <svg data-testid="maximize" {...p} />,
  RotateCcw: (p: Record<string, unknown>) => <svg data-testid="rotate-ccw" {...p} />
}))

import { RunEntry } from '../src/renderer/components/workflow-editor/RunEntry'
import type { WorkflowExecution, WorkflowNode, NodeExecutionState } from '../src/shared/types'

function makeExec(overrides: Partial<WorkflowExecution> = {}): WorkflowExecution {
  return {
    workflowId: 'wf-1',
    startedAt: '2026-04-20T10:00:00Z',
    completedAt: '2026-04-20T10:00:05Z',
    status: 'success',
    nodeStates: [],
    ...overrides
  }
}

function makeNode(overrides: Partial<WorkflowNode> = {}): WorkflowNode {
  return {
    id: 'node-1',
    type: 'launchAgent',
    label: 'Run Claude',
    slug: 'run-claude',
    config: {
      agentType: 'claude',
      projectName: 'test',
      projectPath: '/test',
      branch: 'main',
      useWorktree: true,
      headless: true,
      prompt: 'hi'
    },
    position: { x: 0, y: 0 },
    ...overrides
  }
}

function makeState(overrides: Partial<NodeExecutionState> = {}): NodeExecutionState {
  return {
    nodeId: 'node-1',
    status: 'success',
    startedAt: '2026-04-20T10:00:01Z',
    completedAt: '2026-04-20T10:00:05Z',
    logs: 'some streaming logs',
    ...overrides
  }
}

describe('RunEntry', () => {
  it('expands and shows the Resume button when agentSessionId is present', () => {
    const onResume = vi.fn()
    const exec = makeExec({ nodeStates: [makeState({ agentSessionId: 'agent-abc' })] })
    const { getByText, getByLabelText } = render(
      <RunEntry execution={exec} nodes={[makeNode()]} onResumeSession={onResume} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    const resumeBtn = getByLabelText('Resume session')
    fireEvent.click(resumeBtn)
    expect(onResume).toHaveBeenCalledWith('agent-abc', 'claude', 'test', '/test', 'main', true)
  })

  it('calls onViewFullOutput with node logs', () => {
    const onView = vi.fn()
    const exec = makeExec({ nodeStates: [makeState({ agentSessionId: 'agent-abc' })] })
    const { getByText, getByLabelText } = render(
      <RunEntry
        execution={exec}
        nodes={[makeNode()]}
        onViewFullOutput={onView}
        onResumeSession={vi.fn()}
      />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    fireEvent.click(getByLabelText('View full output'))
    expect(onView).toHaveBeenCalledWith('some streaming logs')
  })

  it('uses the resolved agent/project captured in node state (fromTask sentinel)', () => {
    const onResume = vi.fn()
    const exec = makeExec({
      triggerTaskId: 'task-1',
      nodeStates: [
        makeState({
          agentSessionId: 'agent-abc',
          agentType: 'claude',
          projectName: 'from-task',
          projectPath: '/abs/from-task',
          taskId: 'task-1'
        })
      ]
    })
    const fromTaskNode = makeNode({
      config: {
        agentType: 'fromTask',
        projectName: '',
        projectPath: '',
        headless: true,
        prompt: 'hi'
      }
    })
    const { getByText, getByLabelText } = render(
      <RunEntry execution={exec} nodes={[fromTaskNode]} onResumeSession={onResume} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    fireEvent.click(getByLabelText('Resume session'))
    expect(onResume).toHaveBeenCalledWith(
      'agent-abc',
      'claude',
      'from-task',
      '/abs/from-task',
      undefined,
      undefined
    )
  })

  it('hides Resume when project cannot be resolved (fromTask node without recorded state)', () => {
    const fromTaskNode = makeNode({
      config: {
        agentType: 'fromTask',
        projectName: '',
        projectPath: '',
        headless: true,
        prompt: 'hi'
      }
    })
    const exec = makeExec({ nodeStates: [makeState({ agentSessionId: 'agent-abc' })] })
    const { getByText, queryByLabelText } = render(
      <RunEntry execution={exec} nodes={[fromTaskNode]} onResumeSession={vi.fn()} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(queryByLabelText('Resume session')).not.toBeInTheDocument()
  })

  it('hides Resume button for unsupported agents (gemini)', () => {
    const exec = makeExec({ nodeStates: [makeState({ agentSessionId: 'agent-abc' })] })
    const geminiNode = makeNode({
      config: {
        agentType: 'gemini',
        projectName: 'test',
        projectPath: '/test',
        headless: true,
        prompt: 'hi'
      }
    })
    const { getByText, queryByLabelText } = render(
      <RunEntry execution={exec} nodes={[geminiNode]} onResumeSession={vi.fn()} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(queryByLabelText('Resume session')).not.toBeInTheDocument()
  })

  it('shows Resume button on error-only branch (no logs, just error)', () => {
    const onResume = vi.fn()
    const exec = makeExec({
      status: 'error',
      nodeStates: [
        makeState({
          status: 'error',
          agentSessionId: 'agent-abc',
          logs: undefined,
          error: 'boom'
        })
      ]
    })
    const { getByText, getByLabelText } = render(
      <RunEntry execution={exec} nodes={[makeNode()]} onResumeSession={onResume} />
    )
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    fireEvent.click(getByLabelText('Resume session'))
    expect(onResume).toHaveBeenCalled()
  })

  it('shows the running empty state when an expanded step has no output yet', () => {
    const exec = makeExec({
      status: 'running',
      completedAt: undefined,
      nodeStates: [makeState({ status: 'running', logs: undefined })]
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(getByText(/No output captured yet/)).toBeInTheDocument()
  })

  it("shows the pending empty state for a step that hasn't started", () => {
    const exec = makeExec({
      status: 'running',
      completedAt: undefined,
      nodeStates: [
        makeState({
          status: 'pending',
          logs: undefined,
          startedAt: undefined,
          completedAt: undefined
        })
      ]
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(getByText(/Step hasn't started yet/)).toBeInTheDocument()
  })

  it('shows the skipped empty state for a skipped step', () => {
    const exec = makeExec({
      status: 'error',
      nodeStates: [makeState({ status: 'skipped', logs: undefined })]
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[makeNode()]} />)
    fireEvent.click(getByText(/ago|just now|seconds/i).closest('button')!)
    fireEvent.click(getByText('Run Claude').closest('button')!)
    expect(getByText(/Step was skipped/)).toBeInTheDocument()
  })
})

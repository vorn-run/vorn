// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
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

vi.mock('../src/renderer/components/ProjectPicker', () => ({
  ProjectPicker: (props: Record<string, unknown>) => (
    <button
      data-testid="project-picker"
      onClick={() => (props.onChange as (n: string) => void)('Vorn')}
    >
      {String(props.currentProject) || 'pick'}
    </button>
  )
}))

const mockExecuteWorkflow = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow: (...args: unknown[]) => mockExecuteWorkflow(...args)
}))

import { useAppStore } from '../src/renderer/stores'
import { SourcePromptDialog } from '../src/renderer/components/SourcePromptDialog'
import type { WorkflowDefinition, WorkflowInputDef } from '../src/shared/types'

function makeWorkflow(
  inputs: WorkflowInputDef[],
  opts: { contextual?: boolean } = {}
): WorkflowDefinition {
  return {
    id: 'wf-inputs',
    name: 'Inputs Workflow',
    icon: 'Zap',
    iconColor: '#fff',
    nodes: [
      {
        id: 'trigger-1',
        type: 'trigger',
        config: { triggerType: 'manual', contextual: opts.contextual, inputs },
        position: { x: 0, y: 0 },
        label: 'Manual'
      },
      {
        id: 'script-1',
        type: 'script',
        config: { scriptType: 'bash', scriptContent: 'echo {{inputs.issue}}' },
        position: { x: 0, y: 0 },
        label: 'Run'
      }
    ],
    edges: [],
    enabled: true,
    workspaceId: 'personal'
  }
}

function openWith(wf: WorkflowDefinition, context?: { task?: unknown; source?: unknown }): void {
  useAppStore.setState({
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    pendingWorkflowRun: { workflowId: wf.id, context } as any,
    config: { ...useAppStore.getState().config!, workflows: [wf] }
  })
}

beforeEach(() => {
  vi.clearAllMocks()
  useAppStore.setState({
    pendingWorkflowRun: null,
    config: {
      projects: [
        { name: 'Vorn', path: '/repo/vorn', preferredAgents: [], icon: '', iconColor: '' }
      ],
      workflows: [],
      defaults: { defaultAgent: 'claude' as const, rowHeight: 208 },
      remoteHosts: [],
      workspaces: [],
      tasks: []
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any
  })
})

describe('SourcePromptDialog — run inputs', () => {
  it('renders a field per declared input', () => {
    openWith(
      makeWorkflow([
        { key: 'issue', label: 'Issue URL', type: 'text' },
        { key: 'notes', label: 'Notes', type: 'textarea' }
      ])
    )
    render(<SourcePromptDialog />)
    expect(screen.getByText('Issue URL')).toBeInTheDocument()
    expect(screen.getByText('Notes')).toBeInTheDocument()
  })

  it('does not ask for a source when the workflow is not contextual', () => {
    openWith(makeWorkflow([{ key: 'issue', label: 'Issue URL', type: 'text' }]))
    render(<SourcePromptDialog />)
    expect(screen.queryByTestId('project-picker')).not.toBeInTheDocument()
  })

  it('passes entered values to executeWorkflow as context.inputs', () => {
    openWith(makeWorkflow([{ key: 'issue', label: 'Issue URL', type: 'text' }]))
    render(<SourcePromptDialog />)

    fireEvent.change(screen.getByLabelText('Issue URL'), { target: { value: 'gh-42' } })
    fireEvent.click(screen.getByText('Run'))

    expect(mockExecuteWorkflow).toHaveBeenCalledTimes(1)
    const [, context] = mockExecuteWorkflow.mock.calls[0]
    expect(context).toEqual({ inputs: { issue: 'gh-42' } })
  })

  it('seeds defaults and submits them untouched', () => {
    openWith(makeWorkflow([{ key: 'branch', label: 'Branch', type: 'text', defaultValue: 'main' }]))
    render(<SourcePromptDialog />)
    fireEvent.click(screen.getByText('Run'))

    const [, context] = mockExecuteWorkflow.mock.calls[0]
    expect(context).toEqual({ inputs: { branch: 'main' } })
  })

  it('blocks Run until every required input has a value', () => {
    openWith(makeWorkflow([{ key: 'issue', label: 'Issue URL', type: 'text', required: true }]))
    render(<SourcePromptDialog />)

    const run = screen.getByText('Run')
    expect(run).toBeDisabled()

    fireEvent.change(screen.getByLabelText('Issue URL'), { target: { value: '42' } })
    expect(run).not.toBeDisabled()
  })

  it('treats an unchecked toggle as answered, not missing', () => {
    openWith(makeWorkflow([{ key: 'force', label: 'Force', type: 'boolean', required: true }]))
    render(<SourcePromptDialog />)

    fireEvent.click(screen.getByText('Run'))
    const [, context] = mockExecuteWorkflow.mock.calls[0]
    expect(context).toEqual({ inputs: { force: false } })
  })

  it('keeps the caller-supplied context and adds inputs to it', () => {
    const wf = makeWorkflow([{ key: 'issue', label: 'Issue URL', type: 'text' }], {
      contextual: true
    })
    const task = { id: 'task-1', title: 'Fix bug' }
    openWith(wf, { task })
    render(<SourcePromptDialog />)

    // The caller already knows the folder, so the dialog must not re-ask.
    expect(screen.queryByTestId('project-picker')).not.toBeInTheDocument()

    fireEvent.change(screen.getByLabelText('Issue URL'), { target: { value: 'gh-7' } })
    fireEvent.click(screen.getByText('Run'))

    const [, context] = mockExecuteWorkflow.mock.calls[0]
    expect(context).toEqual({ task, inputs: { issue: 'gh-7' } })
  })

  it('clears entered values when the dialog reopens for another workflow', () => {
    openWith(makeWorkflow([{ key: 'issue', label: 'Issue URL', type: 'text' }]))
    const { rerender } = render(<SourcePromptDialog />)
    fireEvent.change(screen.getByLabelText('Issue URL'), { target: { value: 'stale' } })

    const other = { ...makeWorkflow([{ key: 'issue', label: 'Issue URL', type: 'text' }]) }
    other.id = 'wf-other'
    act(() => openWith(other))
    rerender(<SourcePromptDialog />)

    expect((screen.getByLabelText('Issue URL') as HTMLInputElement).value).toBe('')
  })
})

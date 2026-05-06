// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
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

const projectPickerProps: Array<Record<string, unknown>> = []
vi.mock('../src/renderer/components/ProjectPicker', () => ({
  ProjectPicker: (props: Record<string, unknown>) => {
    projectPickerProps.push(props)
    return (
      <button
        data-testid="project-picker"
        onClick={() => (props.onChange as (n: string) => void)('Vorn')}
      >
        {String(props.currentProject) || 'pick'}
      </button>
    )
  }
}))

const mockExecuteWorkflow = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow: (...args: unknown[]) => mockExecuteWorkflow(...args)
}))

import { useAppStore } from '../src/renderer/stores'
import { SourcePromptDialog } from '../src/renderer/components/SourcePromptDialog'
import type { WorkflowDefinition } from '../src/shared/types'

function makeWorkflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: 'wf-context',
    name: 'Contextual Workflow',
    icon: 'Zap',
    iconColor: '#fff',
    nodes: [
      {
        id: 'trigger-1',
        type: 'trigger',
        config: { triggerType: 'manual', contextual: true },
        position: { x: 0, y: 0 },
        label: 'Manual'
      },
      {
        id: 'launch-1',
        type: 'launchAgent',
        config: {
          agentType: 'claude',
          projectName: '{{context.projectName}}',
          projectPath: '{{context.projectPath}}'
        },
        position: { x: 0, y: 0 },
        label: 'Run agent'
      }
    ],
    edges: [],
    enabled: true,
    workspaceId: 'personal',
    ...overrides
  }
}

beforeEach(() => {
  vi.clearAllMocks()
  projectPickerProps.length = 0
  useAppStore.setState({
    pendingContextualWorkflowId: null,
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

describe('SourcePromptDialog', () => {
  it('renders nothing when no contextual workflow is pending', () => {
    const { container } = render(<SourcePromptDialog />)
    expect(container.textContent).toBe('')
  })

  it('renders the workflow name in its title when pending', () => {
    const wf = makeWorkflow()
    useAppStore.setState({
      pendingContextualWorkflowId: wf.id,
      config: {
        ...useAppStore.getState().config!,
        workflows: [wf]
      }
    })
    render(<SourcePromptDialog />)
    expect(screen.getByText(/Run "Contextual Workflow"/)).toBeInTheDocument()
  })

  it('asks for a branch only when the workflow references {{context.branch}}', () => {
    const wf = makeWorkflow({
      nodes: [
        {
          id: 't',
          type: 'trigger',
          config: { triggerType: 'manual', contextual: true },
          position: { x: 0, y: 0 },
          label: 'Manual'
        },
        {
          id: 'a',
          type: 'launchAgent',
          config: {
            agentType: 'claude',
            projectName: '{{context.projectName}}',
            projectPath: '{{context.projectPath}}',
            branch: '{{context.branch}}'
          },
          position: { x: 0, y: 0 },
          label: 'Run'
        }
      ]
    })
    useAppStore.setState({
      pendingContextualWorkflowId: wf.id,
      config: { ...useAppStore.getState().config!, workflows: [wf] }
    })
    render(<SourcePromptDialog />)
    expect(screen.getByText('Branch')).toBeInTheDocument()
  })

  it('hides the branch input when nothing references {{context.branch}}', () => {
    const wf = makeWorkflow()
    useAppStore.setState({
      pendingContextualWorkflowId: wf.id,
      config: { ...useAppStore.getState().config!, workflows: [wf] }
    })
    render(<SourcePromptDialog />)
    expect(screen.queryByText('Branch')).not.toBeInTheDocument()
  })

  it('shows a worktree checkbox when an agent uses useWorktree: fromContext', () => {
    const wf = makeWorkflow({
      nodes: [
        {
          id: 't',
          type: 'trigger',
          config: { triggerType: 'manual', contextual: true },
          position: { x: 0, y: 0 },
          label: 'Manual'
        },
        {
          id: 'a',
          type: 'launchAgent',
          config: {
            agentType: 'claude',
            projectName: '{{context.projectName}}',
            projectPath: '{{context.projectPath}}',
            useWorktree: 'fromContext'
          },
          position: { x: 0, y: 0 },
          label: 'Run'
        }
      ]
    })
    useAppStore.setState({
      pendingContextualWorkflowId: wf.id,
      config: { ...useAppStore.getState().config!, workflows: [wf] }
    })
    render(<SourcePromptDialog />)
    expect(screen.getByText('Run in a new worktree')).toBeInTheDocument()
  })

  it('clears the pending id and does not run on Cancel', () => {
    const wf = makeWorkflow()
    useAppStore.setState({
      pendingContextualWorkflowId: wf.id,
      config: { ...useAppStore.getState().config!, workflows: [wf] }
    })
    render(<SourcePromptDialog />)
    fireEvent.click(screen.getByText('Cancel'))
    expect(useAppStore.getState().pendingContextualWorkflowId).toBeNull()
    expect(mockExecuteWorkflow).not.toHaveBeenCalled()
  })

  it('runs executeWorkflow with a synthesized source on submit', () => {
    const wf = makeWorkflow()
    useAppStore.setState({
      pendingContextualWorkflowId: wf.id,
      config: { ...useAppStore.getState().config!, workflows: [wf] }
    })
    render(<SourcePromptDialog />)
    // The default-seeded effect picks the first project automatically; click
    // Run directly.
    fireEvent.click(screen.getByText('Run'))
    expect(mockExecuteWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'wf-context' }),
      expect.objectContaining({
        source: expect.objectContaining({
          projectName: 'Vorn',
          projectPath: '/repo/vorn'
        })
      }),
      { source: 'manual' }
    )
    expect(useAppStore.getState().pendingContextualWorkflowId).toBeNull()
  })

  it('detects context references in script nodes (cwd / projectName / projectPath)', () => {
    const wf = makeWorkflow({
      nodes: [
        {
          id: 't',
          type: 'trigger',
          config: { triggerType: 'manual', contextual: true },
          position: { x: 0, y: 0 },
          label: 'Manual'
        },
        {
          id: 's',
          type: 'script',
          config: {
            scriptType: 'bash',
            scriptContent: 'cd {{context.cwd}} && pwd',
            cwd: '{{context.cwd}}',
            projectName: '{{context.projectName}}',
            projectPath: '{{context.projectPath}}'
          },
          position: { x: 0, y: 0 },
          label: 'Run'
        }
      ]
    })
    useAppStore.setState({
      pendingContextualWorkflowId: wf.id,
      config: { ...useAppStore.getState().config!, workflows: [wf] }
    })
    render(<SourcePromptDialog />)
    expect(screen.getByText('Project')).toBeInTheDocument()
  })

  it('detects branch references inside a script body', () => {
    const wf = makeWorkflow({
      nodes: [
        {
          id: 't',
          type: 'trigger',
          config: { triggerType: 'manual', contextual: true },
          position: { x: 0, y: 0 },
          label: 'Manual'
        },
        {
          id: 's',
          type: 'script',
          config: {
            scriptType: 'bash',
            scriptContent: 'git checkout {{context.branch}}'
          },
          position: { x: 0, y: 0 },
          label: 'Run'
        }
      ]
    })
    useAppStore.setState({
      pendingContextualWorkflowId: wf.id,
      config: { ...useAppStore.getState().config!, workflows: [wf] }
    })
    render(<SourcePromptDialog />)
    expect(screen.getByText('Branch')).toBeInTheDocument()
  })
})

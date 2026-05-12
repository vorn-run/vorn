// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { HeadlessSession, WorkflowDefinition } from '../src/shared/types'

vi.mock('../src/renderer/components/AgentIcon', () => ({
  AgentIcon: () => <span data-testid="agent-icon" />
}))

import { useAppStore } from '../src/renderer/stores'
import { HeadlessPill } from '../src/renderer/components/HeadlessPill'

const baseSession: HeadlessSession = {
  id: 'h-1',
  pid: 1234,
  agentType: 'claude',
  projectName: 'demo',
  projectPath: '/demo',
  status: 'running',
  startedAt: Date.now() - 5_000
}

const workflow: WorkflowDefinition = {
  id: 'wf-1',
  name: 'My Workflow',
  icon: 'workflow',
  iconColor: '#aabbcc',
  nodes: [],
  edges: []
}

const initialState = useAppStore.getState()

describe('HeadlessPill', () => {
  beforeEach(() => {
    useAppStore.setState({
      ...initialState,
      headlessLastOutput: new Map(),
      config: { ...(initialState.config ?? {}), workflows: [workflow] } as never
    })
  })

  afterEach(() => {
    useAppStore.setState(initialState)
  })

  it('renders project name and no workflow tag when session has no workflow', () => {
    render(<HeadlessPill session={baseSession} />)
    expect(screen.getByText('demo')).toBeInTheDocument()
    expect(screen.queryByText('My Workflow')).not.toBeInTheDocument()
  })

  it('renders the workflow tag when session is workflow-launched', () => {
    const session: HeadlessSession = {
      ...baseSession,
      workflowId: workflow.id,
      workflowName: workflow.name
    }
    render(<HeadlessPill session={session} />)
    expect(screen.getByText('My Workflow')).toBeInTheDocument()
  })

  it('opens the workflow editor when the tag is clicked', () => {
    const setEditing = vi.fn()
    const setOpen = vi.fn()
    useAppStore.setState({ setEditingWorkflowId: setEditing, setWorkflowEditorOpen: setOpen })
    const session: HeadlessSession = {
      ...baseSession,
      workflowId: workflow.id,
      workflowName: workflow.name
    }
    render(<HeadlessPill session={session} />)
    fireEvent.click(screen.getByText('My Workflow'))
    expect(setEditing).toHaveBeenCalledWith(workflow.id)
    expect(setOpen).toHaveBeenCalledWith(true)
  })
})

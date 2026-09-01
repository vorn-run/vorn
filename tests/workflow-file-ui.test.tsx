// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { useAppStore } from '../src/renderer/stores'
import { WorkflowsSection } from '../src/renderer/components/project-sidebar/WorkflowsSection'
import { __resetConnectionsCacheForTests } from '../src/renderer/lib/use-connections'
import { fileFromWorkflow } from '../src/renderer/lib/workflow-files'
import type { AppConfig, WorkflowDefinition } from '../packages/shared/src/types'

const PROJECT = { name: 'Novum', path: '/Users/someone/dev/novum' }

function workflow(): WorkflowDefinition {
  return {
    id: 'wf-1',
    name: 'Nightly digest',
    icon: 'Zap',
    iconColor: '#6366f1',
    enabled: true,
    workspaceId: 'personal',
    nodes: [
      {
        id: 'trigger-1',
        type: 'trigger',
        label: 'Manual',
        config: { triggerType: 'manual' },
        position: { x: 0, y: 0 }
      }
    ],
    edges: []
  }
}

const initialState = useAppStore.getState()
const addWorkflow = vi.fn()
const updateWorkflow = vi.fn()

beforeEach(() => {
  __resetConnectionsCacheForTests()
  addWorkflow.mockClear()
  updateWorkflow.mockClear()
  ;(window as unknown as { api: unknown }).api = {
    listConnections: vi.fn().mockResolvedValue([]),
    onConfigChanged: vi.fn().mockReturnValue(() => {}),
    saveTextFile: vi.fn().mockResolvedValue('/tmp/nightly-digest.vorn-workflow.json')
  }
  act(() => {
    useAppStore.setState({
      config: {
        ...(initialState.config ?? {}),
        projects: [{ ...PROJECT, preferredAgents: [] }],
        workflows: [workflow()]
      } as AppConfig,
      activeProject: PROJECT.name,
      activeWorkspace: 'personal',
      addWorkflow,
      updateWorkflow
    })
  })
})

afterEach(() => {
  act(() => useAppStore.setState(initialState, true))
})

/** A file drop as the browser delivers one. */
function dropFile(contents: string, name = 'nightly-digest.vorn-workflow.json'): void {
  const file = new File([contents], name, { type: 'application/json' })
  fireEvent.drop(screen.getByLabelText('All runs'), {
    dataTransfer: { files: [file], types: ['Files'] }
  })
}

describe('bringing a workflow file in', () => {
  it('offers importing beside creating', () => {
    render(<WorkflowsSection isCollapsed={false} workspaceWorkflows={[workflow()]} />)
    expect(screen.getByLabelText('Import workflow file')).toBeInTheDocument()
    expect(screen.getByLabelText('New workflow')).toBeInTheDocument()
  })

  it('adds the workflow a dropped file describes', async () => {
    render(<WorkflowsSection isCollapsed={false} workspaceWorkflows={[workflow()]} />)
    const exported = fileFromWorkflow(workflow(), PROJECT.path, []).contents

    dropFile(exported)

    await waitFor(() => expect(addWorkflow).toHaveBeenCalledTimes(1))
    const added = addWorkflow.mock.calls[0][0] as WorkflowDefinition
    expect(added.name).toBe('Nightly digest')
    // Derived from the file, so dropping it again updates rather than duplicates.
    expect(added.id).toBe('import:novum:nightly-digest')
    expect(added.workspaceId).toBe('personal')
  })

  it('updates in place when that file has been dropped before', async () => {
    const exported = fileFromWorkflow(workflow(), PROJECT.path, []).contents
    const already = { ...workflow(), id: 'import:novum:nightly-digest', workspaceId: 'team' }
    act(() => {
      useAppStore.setState({
        config: {
          ...(useAppStore.getState().config as AppConfig),
          workflows: [already]
        } as AppConfig
      })
    })

    render(<WorkflowsSection isCollapsed={false} workspaceWorkflows={[already]} />)
    dropFile(exported)

    await waitFor(() => expect(updateWorkflow).toHaveBeenCalledTimes(1))
    expect(addWorkflow).not.toHaveBeenCalled()
    // Re-import must not move a workflow out of the workspace it lives in.
    expect((updateWorkflow.mock.calls[0][1] as WorkflowDefinition).workspaceId).toBe('team')
  })

  it('says so rather than importing when the file is not a workflow', async () => {
    render(<WorkflowsSection isCollapsed={false} workspaceWorkflows={[workflow()]} />)

    dropFile('this is not json')

    await waitFor(() => expect(screen.queryByText(/not valid JSON/)).toBeTruthy(), {
      timeout: 1000
    }).catch(() => {})
    expect(addWorkflow).not.toHaveBeenCalled()
  })

  it('ignores a drop carrying no file it can read', async () => {
    render(<WorkflowsSection isCollapsed={false} workspaceWorkflows={[workflow()]} />)

    const file = new File(['x'], 'notes.txt', { type: 'text/plain' })
    fireEvent.drop(screen.getByLabelText('All runs'), {
      dataTransfer: { files: [file], types: ['Files'] }
    })

    await waitFor(() => expect(addWorkflow).not.toHaveBeenCalled())
  })
})

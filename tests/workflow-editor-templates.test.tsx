// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowDefinition } from '../src/shared/types'

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

const captured = vi.hoisted(() => ({
  canvasProps: null as Record<string, unknown> | null,
  propertiesProps: null as Record<string, unknown> | null
}))
vi.mock('../src/renderer/components/workflow-editor/WorkflowCanvas', () => ({
  WorkflowCanvas: (props: Record<string, unknown>) => {
    captured.canvasProps = props
    return <div data-testid="canvas" />
  }
}))

vi.mock('../src/renderer/components/workflow-editor/panels/StepLibrary', () => ({
  StepLibrary: () => <div data-testid="step-library" />
}))
vi.mock('../src/renderer/components/workflow-editor/panels/NodeConfigPanel', () => ({
  NodeConfigPanel: () => <div data-testid="node-config" />
}))
vi.mock('../src/renderer/components/workflow-editor/panels/RunHistoryPanel', () => ({
  RunHistoryPanel: () => <div data-testid="run-history" />
}))
vi.mock('../src/renderer/components/workflow-editor/panels/WorkflowPropertiesPanel', () => ({
  WorkflowPropertiesPanel: (props: Record<string, unknown>) => {
    captured.propertiesProps = props
    return <div data-testid="properties-panel" />
  }
}))
vi.mock('../src/renderer/lib/workflow-execution', async (importActual) => ({
  ...(await importActual<Record<string, unknown>>()),
  executeWorkflow: vi.fn().mockResolvedValue(undefined)
}))

const toasts = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
  loading: vi.fn(),
  dismiss: vi.fn(),
  update: vi.fn()
}))
vi.mock('../src/renderer/components/Toast', () => ({
  toast: Object.assign(vi.fn(), toasts)
}))

const EXPORTABLE: WorkflowDefinition = {
  id: 'wf-1',
  name: 'Nightly digest',
  icon: 'Zap',
  iconColor: '#6366f1',
  enabled: true,
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

const mockState = {
  isWorkflowEditorOpen: true,
  editingWorkflowId: null as string | null,
  setWorkflowEditorOpen: vi.fn(),
  setEditingWorkflowId: vi.fn(),
  addWorkflow: vi.fn(),
  updateWorkflow: vi.fn(),
  removeWorkflow: vi.fn(),
  config: {
    workflows: [EXPORTABLE],
    tasks: [],
    projects: [{ name: 'Novum', path: '/Users/someone/dev/novum', preferredAgents: [] }],
    defaults: {}
  },
  activeProject: 'Novum',
  setPendingWorkflowRun: vi.fn(),
  addTerminal: vi.fn(),
  setFocusedTerminal: vi.fn(),
  setSelectedTaskId: vi.fn(),
  activeWorkspace: 'personal',
  workflowExecutions: new Map<string, unknown>()
}

vi.mock('../src/renderer/stores', () => {
  const useAppStore = (selector?: (s: unknown) => unknown) =>
    selector ? selector(mockState) : mockState
  useAppStore.getState = () => mockState
  return { useAppStore }
})

const seedConnectorWorkflow = vi.fn().mockResolvedValue({ workflowId: 'seeded-1', created: true })
const saveTextFile = vi.fn().mockResolvedValue('/tmp/nightly-digest.vorn-workflow.json')

const CONNECTION = {
  id: 'conn-1',
  name: 'workspace-eng',
  connectorId: 'github',
  filters: {},
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-09-01T00:00:00Z'
}

;(window as unknown as { api: object }).api = {
  listWorkflowRuns: vi.fn().mockResolvedValue([]),
  isWindowMaximized: vi.fn().mockResolvedValue(false),
  onWindowMaximizedChange: vi.fn(() => () => {}),
  onConfigChanged: vi.fn(() => () => {}),
  listConnections: vi.fn().mockResolvedValue([CONNECTION]),
  listConnectorPacks: vi.fn().mockResolvedValue([]),
  listConnectors: vi.fn().mockResolvedValue([
    {
      id: 'github',
      name: 'GitHub',
      manifest: {
        defaultWorkflows: [
          { name: 'New issues to tasks', event: 'issueCreated', defaultCronFromMinutes: 5 }
        ]
      }
    }
  ]),
  listConnectorCatalog: vi.fn(),
  listConnectionActions: vi.fn().mockResolvedValue([]),
  seedConnectorWorkflow,
  saveTextFile
}

const { TEMPLATE_SEED } = await import('../packages/server/src/connectors/template-seed')
const { __resetConnectionsCacheForTests } = await import('../src/renderer/lib/use-connections')
const { WorkflowEditor } = await import('../src/renderer/components/workflow-editor/WorkflowEditor')

const api = (window as unknown as { api: Record<string, ReturnType<typeof vi.fn>> }).api

beforeEach(() => {
  __resetConnectionsCacheForTests()
  vi.clearAllMocks()
  api.listWorkflowRuns.mockResolvedValue([])
  api.listConnections.mockResolvedValue([CONNECTION])
  api.listConnectorPacks.mockResolvedValue([])
  api.listConnectorCatalog.mockResolvedValue({
    items: [],
    templates: TEMPLATE_SEED,
    mcpServers: []
  })
  api.listConnectors.mockResolvedValue([
    {
      id: 'github',
      name: 'GitHub',
      manifest: {
        defaultWorkflows: [
          { name: 'New issues to tasks', event: 'issueCreated', defaultCronFromMinutes: 5 }
        ]
      }
    }
  ])
  seedConnectorWorkflow.mockResolvedValue({ workflowId: 'seeded-1', created: true })
  saveTextFile.mockResolvedValue('/tmp/nightly-digest.vorn-workflow.json')
})

afterEach(() => {
  mockState.editingWorkflowId = null
  captured.canvasProps = null
  captured.propertiesProps = null
})

describe('the one panel a new workflow opens with', () => {
  it('asks what to start from, and does not offer settings beside it', () => {
    const { container, queryByTestId } = render(<WorkflowEditor />)
    expect(container.querySelector('[data-start-from]')).toBeTruthy()
    expect(queryByTestId('properties-panel')).not.toBeInTheDocument()
  })

  it('hands the slot to settings when they are asked for', async () => {
    render(<WorkflowEditor />)
    await act(async () => {
      fireEvent.click(screen.getByLabelText('More options'))
    })
    fireEvent.click(screen.getByText('Workflow settings'))

    expect(screen.getByTestId('properties-panel')).toBeInTheDocument()
    expect(document.querySelector('[data-start-from]')).toBeNull()
  })

  it('gives the slot back to settings once the canvas has something on it', async () => {
    const { container } = render(<WorkflowEditor />)
    fireEvent.click(await screen.findByText('Morning digest'))

    await waitFor(() => expect(container.querySelector('[data-start-from]')).toBeNull())
    expect(screen.getByTestId('properties-panel')).toBeInTheDocument()
  })
})

describe('a new workflow carries none of the last one', () => {
  const RUN = {
    runId: 'run-1',
    workflowId: 'wf-1',
    startedAt: '2026-09-01T08:00:00Z',
    completedAt: '2026-09-01T08:01:00Z',
    status: 'success' as const,
    nodeStates: []
  }

  it('forgets the previous workflow runs when New is opened', async () => {
    mockState.editingWorkflowId = 'wf-1'
    api.listWorkflowRuns.mockResolvedValue([RUN])
    const { rerender } = render(<WorkflowEditor />)
    await waitFor(() => expect(captured.propertiesProps?.lastRun).toBeTruthy())

    mockState.editingWorkflowId = null
    rerender(<WorkflowEditor />)
    await act(async () => {
      fireEvent.click(screen.getByLabelText('More options'))
    })
    fireEvent.click(screen.getByText('Workflow settings'))

    // The last run belonged to the workflow that was open, not to this one.
    await waitFor(() => expect(captured.propertiesProps?.lastRun).toBeNull())
  })

  it('stops counting the previous workflow runs in the toolbar', async () => {
    mockState.editingWorkflowId = 'wf-1'
    api.listWorkflowRuns.mockResolvedValue([RUN])
    const { rerender } = render(<WorkflowEditor />)
    await screen.findByRole('button', { name: /Run history \(1\)/ })

    mockState.editingWorkflowId = null
    rerender(<WorkflowEditor />)

    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /Run history \(1\)/ })).toBeNull()
    )
  })
})

describe('starting a new workflow from a template', () => {
  it('offers the published templates on an empty canvas', async () => {
    render(<WorkflowEditor />)
    expect(await screen.findByText('Webhook to report')).toBeInTheDocument()
    expect(screen.getByText('Blank canvas')).toBeInTheDocument()
  })

  it('puts the template on the canvas and names the workflow after it', async () => {
    const { container } = render(<WorkflowEditor />)
    fireEvent.click(await screen.findByText('Morning digest'))

    await waitFor(() => {
      expect((captured.canvasProps?.nodes as unknown[]).length).toBe(3)
    })
    const nameInput = container.querySelector(
      'input[placeholder="Workflow name"]'
    ) as HTMLInputElement
    expect(nameInput.value).toBe('Morning digest')
    // Picking answers the question the panel asked, so it goes away.
    expect(screen.queryByText('Blank canvas')).not.toBeInTheDocument()
  })

  it('resolves the template against the project in view', async () => {
    render(<WorkflowEditor />)
    fireEvent.click(await screen.findByText('Morning digest'))

    await waitFor(() => expect(captured.canvasProps?.nodes).toBeTruthy())
    const nodes = captured.canvasProps?.nodes as Array<{
      id: string
      config: Record<string, unknown>
    }>
    expect(nodes.find((n) => n.id === 'gather')?.config.projectPath).toBe(
      '/Users/someone/dev/novum'
    )
  })

  it('warns about what the template still needs here', async () => {
    render(<WorkflowEditor />)
    fireEvent.click(await screen.findByText('Webhook to report'))

    await waitFor(() => expect(toasts.warning).toHaveBeenCalledTimes(1))
    expect(toasts.warning.mock.calls[0][0]).toContain('HTTP profile')
  })

  it('leaves the canvas alone when the blank one is chosen', async () => {
    render(<WorkflowEditor />)
    fireEvent.click(await screen.findByText('Blank canvas'))

    await waitFor(() => expect(screen.queryByText('Blank canvas')).not.toBeInTheDocument())
    expect((captured.canvasProps?.nodes as unknown[]).length).toBe(0)
  })
})

describe('starting from what a connection already builds', () => {
  it('asks the server for it and opens what comes back', async () => {
    render(<WorkflowEditor />)
    fireEvent.click(await screen.findByText('New issues to tasks'))

    await waitFor(() =>
      expect(seedConnectorWorkflow).toHaveBeenCalledWith('conn-1', 'issueCreated')
    )
    expect(mockState.setEditingWorkflowId).toHaveBeenCalledWith('seeded-1')
    expect(toasts.success).toHaveBeenCalled()
  })

  it('says so when the server refuses', async () => {
    seedConnectorWorkflow.mockRejectedValue(new Error('no such connection'))
    render(<WorkflowEditor />)
    fireEvent.click(await screen.findByText('New issues to tasks'))

    await waitFor(() => expect(toasts.error).toHaveBeenCalledWith('no such connection'))
    expect(mockState.setEditingWorkflowId).not.toHaveBeenCalled()
  })
})

describe('exporting the workflow being edited', () => {
  async function openMenu(): Promise<void> {
    mockState.editingWorkflowId = 'wf-1'
    render(<WorkflowEditor />)
    await act(async () => {
      fireEvent.click(screen.getByLabelText('More options'))
    })
  }

  it('writes the file the canvas describes', async () => {
    await openMenu()
    fireEvent.click(screen.getByText('Export as file…'))

    await waitFor(() => expect(saveTextFile).toHaveBeenCalledTimes(1))
    expect(saveTextFile.mock.calls[0][0].defaultName).toBe('nightly-digest.vorn-workflow.json')
    expect(JSON.parse(saveTextFile.mock.calls[0][0].contents).name).toBe('Nightly digest')
    expect(toasts.success).toHaveBeenCalledWith('Exported "Nightly digest"')
  })

  it('says nothing when the save is cancelled', async () => {
    saveTextFile.mockResolvedValue(null)
    await openMenu()
    fireEvent.click(screen.getByText('Export as file…'))

    await waitFor(() => expect(saveTextFile).toHaveBeenCalledTimes(1))
    expect(toasts.success).not.toHaveBeenCalled()
    expect(toasts.error).not.toHaveBeenCalled()
  })
})

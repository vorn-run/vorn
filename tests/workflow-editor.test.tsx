// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, fireEvent, act } from '@testing-library/react'
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
  libraryProps: null as Record<string, unknown> | null
}))
vi.mock('../src/renderer/components/workflow-editor/WorkflowCanvas', () => ({
  WorkflowCanvas: (props: Record<string, unknown>) => {
    captured.canvasProps = props
    return <div data-testid="canvas" />
  }
}))

vi.mock('../src/renderer/components/workflow-editor/panels/StepLibrary', () => ({
  StepLibrary: (props: Record<string, unknown>) => {
    captured.libraryProps = props
    return <div data-testid="step-library" />
  }
}))

vi.mock('../src/renderer/components/workflow-editor/panels/NodeConfigPanel', () => ({
  NodeConfigPanel: () => <div data-testid="node-config" />
}))

vi.mock('../src/renderer/components/workflow-editor/panels/RunHistoryPanel', () => ({
  RunHistoryPanel: () => <div data-testid="run-history" />
}))

vi.mock('../src/renderer/components/workflow-editor/panels/WorkflowPropertiesPanel', () => ({
  WorkflowPropertiesPanel: () => <div data-testid="properties-panel" />
}))

vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow: vi.fn().mockResolvedValue(undefined)
}))

const mockState = {
  isWorkflowEditorOpen: true,
  editingWorkflowId: null as string | null,
  setWorkflowEditorOpen: vi.fn(),
  setEditingWorkflowId: vi.fn(),
  addWorkflow: vi.fn(),
  updateWorkflow: vi.fn(),
  removeWorkflow: vi.fn(),
  setImportedRequirements: vi.fn(),
  config: { workflows: [] as WorkflowDefinition[], tasks: [], projects: [], defaults: {} },
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
;(global as unknown as { window: object }).window = {
  addEventListener: vi.fn(),
  removeEventListener: vi.fn(),
  api: {
    listWorkflowRuns: vi.fn().mockResolvedValue([]),
    createTerminal: vi.fn(),
    isWindowMaximized: vi.fn().mockResolvedValue(false),
    onWindowMaximizedChange: vi.fn(() => () => {}),
    windowMinimize: vi.fn(),
    windowMaximize: vi.fn(),
    windowClose: vi.fn()
  }
}

const { WorkflowEditor } = await import('../src/renderer/components/workflow-editor/WorkflowEditor')

describe('WorkflowEditor', () => {
  // The editing id decides whether half this toolbar renders at all, and the
  // store mock is a plain object rather than a store — nothing resets it
  // between tests. It was on each test that set it to put it back, which holds
  // right up until one of them forgets and the next test inherits an editor it
  // never asked for.
  afterEach(() => {
    mockState.editingWorkflowId = null
  })

  it('opens a new workflow on the start-from panel rather than its settings', () => {
    // Settings for a workflow that does not exist yet are not the question a
    // blank canvas is asking, and two panels never share the slot.
    const { getByTestId, queryByTestId, container } = render(<WorkflowEditor />)
    expect(getByTestId('canvas')).toBeInTheDocument()
    expect(container.querySelector('[data-start-from]')).toBeTruthy()
    expect(queryByTestId('properties-panel')).not.toBeInTheDocument()
  })

  it('shows the properties panel for a workflow that exists', () => {
    mockState.editingWorkflowId = 'w1'
    const { getByTestId, container } = render(<WorkflowEditor />)
    expect(getByTestId('properties-panel')).toBeInTheDocument()
    expect(container.querySelector('[data-start-from]')).toBeNull()
  })

  it('renders workflow name input', () => {
    const { container } = render(<WorkflowEditor />)
    const nameInput = container.querySelector('input[placeholder="Workflow name"]')
    expect(nameInput).toBeInTheDocument()
  })

  it('renders Save button', () => {
    const { container } = render(<WorkflowEditor />)
    const saveButton = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Save')
    )
    expect(saveButton).toBeDefined()
  })

  it('does not render anything when isOpen is false', () => {
    mockState.isWorkflowEditorOpen = false
    const { container } = render(<WorkflowEditor />)
    expect(container.firstChild).toBeNull()
    mockState.isWorkflowEditorOpen = true
  })

  it('opens the overflow menu when the more button is clicked', () => {
    const { container, getByText } = render(<WorkflowEditor />)
    const moreButton = container.querySelector('svg.lucide-ellipsis')?.closest('button')
    if (moreButton) fireEvent.click(moreButton)
    expect(getByText('Workflow settings')).toBeInTheDocument()
  })

  it('triggers Save when the Save button is clicked', () => {
    const { container } = render(<WorkflowEditor />)
    const saveButton = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Save')
    )
    if (saveButton) fireEvent.click(saveButton)
    expect(mockState.addWorkflow).toHaveBeenCalled()
  })

  it('updates name input', () => {
    const { container } = render(<WorkflowEditor />)
    const nameInput = container.querySelector(
      'input[placeholder="Workflow name"]'
    ) as HTMLInputElement
    fireEvent.change(nameInput, { target: { value: 'New name' } })
    expect(nameInput.value).toBe('New name')
  })

  it('prompts for run inputs instead of launching the workflow blind', async () => {
    // Running straight from the editor used to skip the prompt entirely, so
    // {{inputs.*}} reached the agent unresolved.
    const { executeWorkflow } = await import('../src/renderer/lib/workflow-execution')
    vi.mocked(executeWorkflow).mockClear()
    mockState.setPendingWorkflowRun.mockClear()
    mockState.editingWorkflowId = 'wf-inputs' as unknown as null
    mockState.config.workflows = [
      {
        id: 'wf-inputs',
        name: 'Needs inputs',
        enabled: true,
        nodes: [
          {
            id: 'trigger',
            type: 'trigger',
            label: 'Trigger',
            position: { x: 0, y: 0 },
            config: {
              triggerType: 'manual',
              inputs: [{ key: 'pr_number', label: 'PR number', type: 'text' }]
            }
          }
        ],
        edges: []
      }
    ] as unknown as never[]

    const { container } = render(<WorkflowEditor />)
    fireEvent.click(container.querySelector('button[aria-label="Run workflow"]')!)

    expect(mockState.setPendingWorkflowRun).toHaveBeenCalledWith('wf-inputs', undefined, undefined)
    expect(executeWorkflow).not.toHaveBeenCalled()

    mockState.editingWorkflowId = null
    mockState.config.workflows = []
  })

  it('runs a workflow with no declared inputs directly', async () => {
    const { executeWorkflow } = await import('../src/renderer/lib/workflow-execution')
    vi.mocked(executeWorkflow).mockClear()
    mockState.setPendingWorkflowRun.mockClear()
    mockState.editingWorkflowId = 'wf-plain' as unknown as null
    mockState.config.workflows = [
      {
        id: 'wf-plain',
        name: 'Plain',
        enabled: true,
        nodes: [
          {
            id: 'trigger',
            type: 'trigger',
            label: 'Trigger',
            position: { x: 0, y: 0 },
            config: { triggerType: 'manual' }
          }
        ],
        edges: []
      }
    ] as unknown as never[]

    const { container } = render(<WorkflowEditor />)
    fireEvent.click(container.querySelector('button[aria-label="Run workflow"]')!)

    expect(executeWorkflow).toHaveBeenCalled()
    expect(mockState.setPendingWorkflowRun).not.toHaveBeenCalled()

    mockState.editingWorkflowId = null
    mockState.config.workflows = []
  })

  it('renders the back button which closes the editor', () => {
    const { container } = render(<WorkflowEditor />)
    const backButton = container.querySelector('svg.lucide-arrow-left')?.closest('button')
    if (backButton) fireEvent.click(backButton)
    expect(mockState.setWorkflowEditorOpen).toHaveBeenCalledWith(false)
  })

  it('clicks Delete workflow in the overflow menu when editing', () => {
    mockState.editingWorkflowId = 'w1'
    const { container, getByText } = render(<WorkflowEditor />)
    const moreButton = container.querySelector('svg.lucide-ellipsis')?.closest('button')
    if (moreButton) fireEvent.click(moreButton)
    fireEvent.click(getByText('Delete workflow'))
    expect(mockState.removeWorkflow).toHaveBeenCalledWith('w1')
    mockState.editingWorkflowId = null
  })

  it('keeps Run disabled on a new workflow until a trigger exists', () => {
    mockState.addWorkflow.mockClear()
    const { container } = render(<WorkflowEditor />)
    const playButton = container.querySelector('svg.lucide-play')?.closest('button')
    expect(playButton).toBeDisabled()
    if (playButton) fireEvent.click(playButton)
    expect(mockState.addWorkflow).not.toHaveBeenCalled()
  })

  it('toggles the run history panel via the history toolbar button when editing', () => {
    mockState.editingWorkflowId = 'w1'
    const { getByRole, getByTestId, queryByTestId } = render(<WorkflowEditor />)
    // By the name the button carries, not by the icon inside it. lucide renames
    // icons between releases -- History became an alias for RotateCcwClock in
    // 1.33, so the old class selector matched nothing and the chained
    // `?.closest()` handed back undefined. `toBeDefined()` was the only guard,
    // and undefined is exactly what it fails on, so a renamed icon surfaced as a
    // puzzle instead.
    const historyButton = getByRole('button', { name: /Run history/ })
    fireEvent.click(historyButton)
    expect(getByTestId('run-history')).toBeInTheDocument()
    expect(queryByTestId('properties-panel')).not.toBeInTheDocument()
    fireEvent.click(historyButton)
    expect(queryByTestId('run-history')).not.toBeInTheDocument()
  })

  it('clicks Workflow settings menu item to open properties', () => {
    const { container, getByText, getByTestId } = render(<WorkflowEditor />)
    const moreButton = container.querySelector('svg.lucide-ellipsis')?.closest('button')
    if (moreButton) fireEvent.click(moreButton)
    fireEvent.click(getByText('Workflow settings'))
    expect(getByTestId('properties-panel')).toBeInTheDocument()
  })

  it('opens icon picker when icon button is clicked', () => {
    const { container } = render(<WorkflowEditor />)
    const iconButton = container.querySelector('svg.lucide-workflow')?.closest('button')
    if (iconButton) fireEvent.click(iconButton)
    expect(container.querySelector('.grid')).toBeInTheDocument()
  })

  it('renders inline in the content area when inline=true', () => {
    const { container } = render(<WorkflowEditor inline />)
    const backButton = container.querySelector('svg.lucide-arrow-left')
    expect(backButton).toBeNull()
    const saveButton = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Save')
    )
    expect(saveButton).toBeDefined()
  })

  it('updates an existing workflow on save and preserves lastRun metadata', () => {
    const existing = {
      id: 'w1',
      name: 'Existing',
      icon: 'Zap',
      iconColor: '#ffffff',
      nodes: [
        {
          id: 't',
          type: 'trigger' as const,
          label: 'T',
          config: { triggerType: 'manual' as const },
          position: { x: 0, y: 0 }
        }
      ],
      edges: [],
      enabled: true,
      staggerDelayMs: 500,
      autoCleanupWorktrees: true,
      lastRunAt: '2026-04-20T10:00:00Z',
      lastRunStatus: 'success' as const,
      workspaceId: 'personal'
    }
    mockState.editingWorkflowId = 'w1'
    mockState.config = { workflows: [existing], tasks: [], projects: [], defaults: {} }
    const { container } = render(<WorkflowEditor />)
    const saveButton = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Save')
    )
    if (saveButton) fireEvent.click(saveButton)
    expect(mockState.updateWorkflow).toHaveBeenCalledWith(
      'w1',
      expect.objectContaining({
        id: 'w1',
        staggerDelayMs: 500,
        autoCleanupWorktrees: true,
        lastRunAt: '2026-04-20T10:00:00Z',
        lastRunStatus: 'success',
        workspaceId: 'personal'
      })
    )
    mockState.editingWorkflowId = null
    mockState.config = { workflows: [], tasks: [], projects: [], defaults: {} }
  })
})

describe('the handlers the canvas drives', () => {
  const canvas = () =>
    captured.canvasProps as unknown as {
      nodes: { id: string; type: string; position: { x: number; y: number } }[]
      onConnectEdge: (s: string, t: string) => void
      onPositionsCommit: (p: Record<string, { x: number; y: number }>) => void
      onTidyUp: () => void
      onOpenLibrary: (anchor: Record<string, unknown>) => void
    }
  const save = (container: HTMLElement) => {
    const saveButton = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Save')
    )!
    fireEvent.click(saveButton)
    return mockState.addWorkflow.mock.lastCall![0] as {
      nodes: { id: string; type: string; position: { x: number; y: number }; config: unknown }[]
      edges: { source: string; target: string }[]
    }
  }

  // A new workflow seeds no trigger, so these tests pick one from the library.
  const seedManualTrigger = () => {
    act(() =>
      canvas().onOpenLibrary({
        afterNodeId: '__TRIGGER__',
        beforeNodeId: null,
        insideBranch: false,
        bodyOnly: false
      })
    )
    act(() =>
      (captured.libraryProps!.onPick as (p: unknown) => void)({
        kind: 'triggerType',
        triggerType: 'manual'
      })
    )
  }

  it('writes a hand-drawn connection into the definition', () => {
    mockState.addWorkflow.mockClear()
    const { container } = render(<WorkflowEditor />)
    seedManualTrigger()
    const triggerId = (captured.canvasProps!.nodes as { id: string }[])[0].id
    act(() => canvas().onConnectEdge(triggerId, 'ghost-target'))
    expect(save(container).edges.some((e) => e.target === 'ghost-target')).toBe(true)
  })

  it('persists committed drag positions', () => {
    mockState.addWorkflow.mockClear()
    const { container } = render(<WorkflowEditor />)
    seedManualTrigger()
    const triggerId = (captured.canvasProps!.nodes as { id: string }[])[0].id
    act(() => canvas().onPositionsCommit({ [triggerId]: { x: 48, y: 96 } }))
    expect(save(container).nodes[0].position).toEqual({ x: 48, y: 96 })
  })

  it('tidy up writes the computed layout into the definition', () => {
    mockState.addWorkflow.mockClear()
    const { container } = render(<WorkflowEditor />)
    seedManualTrigger()
    act(() => canvas().onTidyUp())
    // Half a card is 140; the lattice a drag snaps to rounds it to 136.
    expect(save(container).nodes[0].position.x).toBe(-136)
  })

  const pickFromLibrary = (pick: Record<string, unknown>) =>
    act(() => (captured.libraryProps!.onPick as (p: unknown) => void)(pick))

  it('refuses a pick whose anchor no longer exists', () => {
    mockState.addWorkflow.mockClear()
    const { container, getByTestId } = render(<WorkflowEditor />)
    seedManualTrigger()
    act(() =>
      canvas().onOpenLibrary({
        afterNodeId: 'ghost',
        beforeNodeId: null,
        insideBranch: false,
        bodyOnly: false
      })
    )
    expect(getByTestId('step-library')).toBeInTheDocument()
    pickFromLibrary({ kind: 'type', type: 'script' })
    const saved = save(container)
    // No orphan step and no edge hanging off the ghost.
    expect(saved.nodes).toHaveLength(1)
    expect(saved.edges.some((e) => e.source === 'ghost')).toBe(false)
  })

  it('closes the library when its anchor node is deleted', () => {
    const { getByTestId, queryByTestId } = render(<WorkflowEditor />)
    seedManualTrigger()
    const triggerId = (captured.canvasProps!.nodes as { id: string }[])[0].id
    act(() =>
      canvas().onOpenLibrary({
        afterNodeId: triggerId,
        beforeNodeId: null,
        insideBranch: false,
        bodyOnly: false
      })
    )
    expect(getByTestId('step-library')).toBeInTheDocument()
    act(() =>
      (captured.canvasProps as unknown as { onDeleteNode: (id: string) => void }).onDeleteNode(
        triggerId
      )
    )
    expect(queryByTestId('step-library')).toBeNull()
  })

  it('a library pick at a dropped position appends after its anchor there', () => {
    mockState.addWorkflow.mockClear()
    const { container, getByTestId } = render(<WorkflowEditor />)
    seedManualTrigger()
    const triggerId = (captured.canvasProps!.nodes as { id: string }[])[0].id
    act(() =>
      canvas().onOpenLibrary({
        afterNodeId: triggerId,
        beforeNodeId: null,
        insideBranch: false,
        bodyOnly: false,
        position: { x: 24, y: 480 }
      })
    )
    expect(getByTestId('step-library')).toBeInTheDocument()
    pickFromLibrary({ kind: 'type', type: 'script' })
    const saved = save(container)
    const script = saved.nodes.find((n) => n.type === 'script')!
    expect(script.position).toEqual({ x: 24, y: 480 })
    expect(saved.edges.some((e) => e.target === script.id)).toBe(true)
  })

  it('a connector pick lands preconfigured at its anchor', () => {
    mockState.addWorkflow.mockClear()
    const { container } = render(<WorkflowEditor />)
    seedManualTrigger()
    const triggerId = (captured.canvasProps!.nodes as { id: string }[])[0].id
    act(() =>
      canvas().onOpenLibrary({
        afterNodeId: triggerId,
        beforeNodeId: null,
        insideBranch: false,
        bodyOnly: false
      })
    )
    pickFromLibrary({ kind: 'connectorAction', connectionId: 'c9', action: 'createIssue' })
    const saved = save(container)
    const step = saved.nodes.find((n) => n.type === 'callConnectorAction')!
    expect(step.config).toMatchObject({ connectionId: 'c9', action: 'createIssue' })
  })

  it('a parallel pick forks from the anchor', () => {
    mockState.addWorkflow.mockClear()
    const { container } = render(<WorkflowEditor />)
    seedManualTrigger()
    const triggerId = (captured.canvasProps!.nodes as { id: string }[])[0].id
    act(() =>
      canvas().onOpenLibrary({
        afterNodeId: triggerId,
        beforeNodeId: null,
        insideBranch: false,
        bodyOnly: false
      })
    )
    pickFromLibrary({ kind: 'parallel' })
    const saved = save(container)
    expect(saved.nodes.some((n) => n.type === 'launchAgent')).toBe(true)
  })

  it('cmd+z walks an edit back', () => {
    mockState.addWorkflow.mockClear()
    const winAdd = (window.addEventListener as ReturnType<typeof vi.fn>).mock
    const { container } = render(<WorkflowEditor />)
    seedManualTrigger()
    const triggerId = (captured.canvasProps!.nodes as { id: string }[])[0].id
    act(() => canvas().onConnectEdge(triggerId, 'ghost-target'))
    const keydown = winAdd.calls.filter((c) => c[0] === 'keydown').at(-1)![1] as (
      e: unknown
    ) => void
    act(() =>
      keydown({
        metaKey: true,
        ctrlKey: false,
        shiftKey: false,
        key: 'z',
        target: document.createElement('div'),
        preventDefault: () => {}
      })
    )
    expect(save(container).edges).toHaveLength(0)
  })
})

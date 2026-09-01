// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, act } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowDefinition, WorkflowNode } from '../src/shared/types'

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
vi.mock('../src/renderer/components/workflow-editor/WorkflowCanvas', async () => {
  const actual = await vi.importActual<
    typeof import('../src/renderer/components/workflow-editor/WorkflowCanvas')
  >('../src/renderer/components/workflow-editor/WorkflowCanvas')
  return {
    ...actual,
    WorkflowCanvas: (props: Record<string, unknown>) => {
      captured.canvasProps = props
      return <div data-testid="canvas" />
    }
  }
})

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
    isWindowMaximized: vi.fn().mockResolvedValue(false),
    onWindowMaximizedChange: vi.fn(() => () => {})
  }
}

import { TRIGGER_ANCHOR } from '../src/renderer/lib/workflow-canvas-layout'
const { WorkflowEditor } = await import('../src/renderer/components/workflow-editor/WorkflowEditor')

afterEach(() => {
  cleanup()
  mockState.editingWorkflowId = null
  mockState.addWorkflow.mockClear()
  captured.canvasProps = null
  captured.libraryProps = null
})

const canvasNodes = () => (captured.canvasProps?.nodes ?? []) as WorkflowNode[]

function openTriggerLibrary() {
  act(() => {
    ;(captured.canvasProps!.onOpenLibrary as (anchor: unknown) => void)(TRIGGER_ANCHOR)
  })
}

function pickFromLibrary(pick: unknown) {
  act(() => {
    ;(captured.libraryProps!.onPick as (pick: unknown) => void)(pick)
  })
}

describe('the editor without a trigger', () => {
  it('seeds a new workflow with no nodes at all', () => {
    render(<WorkflowEditor />)
    expect(canvasNodes()).toHaveLength(0)
  })

  it('disables Run until a trigger exists', () => {
    const { container } = render(<WorkflowEditor />)
    const runButton = container.querySelector('button[aria-label="Run workflow"]')
    expect(runButton).toBeDisabled()
  })

  it('opens the library in trigger scope from the placeholder anchor', () => {
    render(<WorkflowEditor />)
    openTriggerLibrary()
    expect(captured.libraryProps?.scope).toMatchObject({ triggers: true })
  })

  it('creates the trigger node from a built-in pick and enables Run', () => {
    const { container } = render(<WorkflowEditor />)
    openTriggerLibrary()
    pickFromLibrary({ kind: 'triggerType', triggerType: 'webhook' })

    const trigger = canvasNodes().find((n) => n.type === 'trigger')
    expect(trigger?.config).toMatchObject({ triggerType: 'webhook', method: 'POST' })
    expect((trigger?.config as { token?: string }).token).toBeTruthy()
    expect(container.querySelector('button[aria-label="Run workflow"]')).toBeEnabled()
  })

  it('creates a preconfigured connector poll from a connector event pick', () => {
    render(<WorkflowEditor />)
    openTriggerLibrary()
    pickFromLibrary({ kind: 'connectorTrigger', connectionId: 'c1', event: 'issueCreated' })

    const trigger = canvasNodes().find((n) => n.type === 'trigger')
    expect(trigger?.config).toMatchObject({
      triggerType: 'connectorPoll',
      connectionId: 'c1',
      event: 'issueCreated',
      cron: '*/5 * * * *'
    })
  })

  it('deleting the trigger brings the placeholder back and Run goes dark', () => {
    const { container } = render(<WorkflowEditor />)
    openTriggerLibrary()
    pickFromLibrary({ kind: 'triggerType', triggerType: 'manual' })
    const triggerId = canvasNodes().find((n) => n.type === 'trigger')!.id

    act(() => {
      ;(captured.canvasProps!.onDeleteNode as (id: string) => void)(triggerId)
    })
    expect(canvasNodes().some((n) => n.type === 'trigger')).toBe(false)
    expect(container.querySelector('button[aria-label="Run workflow"]')).toBeDisabled()

    openTriggerLibrary()
    pickFromLibrary({ kind: 'triggerType', triggerType: 'recurring' })
    expect(canvasNodes().some((n) => n.type === 'trigger')).toBe(true)
    expect(container.querySelector('button[aria-label="Run workflow"]')).toBeEnabled()
  })

  it('deleting the trigger keeps downstream steps and drops its edges', () => {
    render(<WorkflowEditor />)
    openTriggerLibrary()
    pickFromLibrary({ kind: 'triggerType', triggerType: 'manual' })
    const triggerId = canvasNodes().find((n) => n.type === 'trigger')!.id

    act(() => {
      ;(captured.canvasProps!.onOpenLibrary as (a: unknown) => void)({
        afterNodeId: triggerId,
        beforeNodeId: null,
        insideBranch: false,
        bodyOnly: false
      })
    })
    pickFromLibrary({ kind: 'type', type: 'script' })
    const script = canvasNodes().find((n) => n.type === 'script')!

    act(() => {
      ;(captured.canvasProps!.onDeleteNode as (id: string) => void)(triggerId)
    })
    const edges = captured.canvasProps!.edges as { source: string; target: string }[]
    expect(canvasNodes().some((n) => n.id === script.id)).toBe(true)
    expect(edges.some((e) => e.source === triggerId || e.target === triggerId)).toBe(false)
  })

  it('hover-replace on the trigger opens trigger scope and swaps in place', () => {
    render(<WorkflowEditor />)
    openTriggerLibrary()
    pickFromLibrary({ kind: 'triggerType', triggerType: 'manual' })
    const triggerId = canvasNodes().find((n) => n.type === 'trigger')!.id

    // The toolbar's replace button sends the same anchor the placeholder does.
    act(() => {
      ;(captured.canvasProps!.onOpenLibrary as (a: unknown) => void)(TRIGGER_ANCHOR)
    })
    expect(captured.libraryProps?.scope).toMatchObject({ triggers: true })
    pickFromLibrary({ kind: 'triggerType', triggerType: 'webhook' })

    const triggers = canvasNodes().filter((n) => n.type === 'trigger')
    expect(triggers).toHaveLength(1)
    expect(triggers[0].id).toBe(triggerId)
    expect(triggers[0].config).toMatchObject({ triggerType: 'webhook' })
  })

  it('replaces the existing trigger in place on a second pick', () => {
    render(<WorkflowEditor />)
    openTriggerLibrary()
    pickFromLibrary({ kind: 'triggerType', triggerType: 'manual' })
    const firstId = canvasNodes().find((n) => n.type === 'trigger')?.id

    openTriggerLibrary()
    pickFromLibrary({ kind: 'triggerType', triggerType: 'recurring' })
    const triggers = canvasNodes().filter((n) => n.type === 'trigger')
    expect(triggers).toHaveLength(1)
    expect(triggers[0].id).toBe(firstId)
    expect(triggers[0].config).toMatchObject({ triggerType: 'recurring', cron: '0 9 * * *' })
    // The old type's label does not survive the swap.
    expect(triggers[0].label).toBe('Schedule (Recurring)')
  })
})

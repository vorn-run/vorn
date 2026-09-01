// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, screen } from '@testing-library/react'
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

const workflow: WorkflowDefinition = {
  id: 'wf-replace',
  name: 'Replace me',
  icon: 'Workflow',
  iconColor: '#fff',
  enabled: true,
  nodes: [
    {
      id: 't',
      type: 'trigger',
      label: 'Trigger',
      config: { triggerType: 'manual' },
      position: { x: 0, y: 0 }
    },
    {
      id: 's1',
      type: 'script',
      label: 'Old script',
      slug: 'old-script',
      config: { scriptType: 'bash', scriptContent: 'echo hi' },
      position: { x: 12, y: 120 }
    },
    {
      id: 's2',
      type: 'script',
      label: 'HTTP Request',
      slug: 'http-request',
      config: { scriptType: 'bash', scriptContent: '' },
      position: { x: 12, y: 240 }
    }
  ] as WorkflowNode[],
  edges: [
    { id: 'e1', source: 't', target: 's1' },
    { id: 'e2', source: 's1', target: 's2' }
  ]
}

const mockState = {
  isWorkflowEditorOpen: true,
  editingWorkflowId: 'wf-replace' as string | null,
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

import { act } from '@testing-library/react'
const { WorkflowEditor } = await import('../src/renderer/components/workflow-editor/WorkflowEditor')

afterEach(() => {
  cleanup()
  captured.canvasProps = null
  captured.libraryProps = null
})

const canvasNodes = () => (captured.canvasProps?.nodes ?? []) as WorkflowNode[]
const canvasEdges = () =>
  (captured.canvasProps?.edges ?? []) as { id: string; source: string; target: string }[]

function openReplace(nodeId: string) {
  act(() => {
    ;(captured.canvasProps!.onOpenLibrary as (a: unknown) => void)({
      afterNodeId: nodeId,
      beforeNodeId: null,
      insideBranch: false,
      bodyOnly: false,
      replaceNodeId: nodeId
    })
  })
}

function pick(p: unknown) {
  act(() => {
    ;(captured.libraryProps!.onPick as (p: unknown) => void)(p)
  })
}

describe('replacing a step in place', () => {
  it('opens the library in replace scope', () => {
    render(<WorkflowEditor />)
    openReplace('s1')
    expect(screen.getByTestId('step-library')).toBeInTheDocument()
    expect(captured.libraryProps?.scope).toMatchObject({ replacing: true })
  })

  it('keeps the id, position, and every edge while swapping type and config', () => {
    render(<WorkflowEditor />)
    openReplace('s1')
    pick({ kind: 'type', type: 'httpRequest' })

    const swapped = canvasNodes().find((n) => n.id === 's1')!
    expect(swapped.type).toBe('httpRequest')
    expect(swapped.config).toMatchObject({ nodeType: 'httpRequest', method: 'GET' })
    expect(swapped.position).toEqual({ x: 12, y: 120 })
    expect(
      canvasEdges()
        .map((e) => e.id)
        .sort()
    ).toEqual(['e1', 'e2'])
    expect(canvasNodes()).toHaveLength(3)
  })

  it('resets the slug to the new type, kept unique against its siblings', () => {
    render(<WorkflowEditor />)
    openReplace('s1')
    pick({ kind: 'type', type: 'httpRequest' })

    const swapped = canvasNodes().find((n) => n.id === 's1')!
    // 'http-request' is taken by s2, so the fresh slug steps around it.
    expect(swapped.slug).toBeTruthy()
    expect(swapped.slug).not.toBe('old-script')
    expect(swapped.slug).not.toBe('http-request')
  })

  it('swaps to a preconfigured connector action', () => {
    render(<WorkflowEditor />)
    openReplace('s2')
    pick({ kind: 'connectorAction', connectionId: 'c1', action: 'createIssue' })

    const swapped = canvasNodes().find((n) => n.id === 's2')!
    expect(swapped.type).toBe('callConnectorAction')
    expect(swapped.config).toMatchObject({ connectionId: 'c1', action: 'createIssue' })
    expect(swapped.position).toEqual({ x: 12, y: 240 })
  })

  it('refuses structural picks in replace mode', () => {
    render(<WorkflowEditor />)
    openReplace('s1')
    pick({ kind: 'type', type: 'condition' })
    const untouched = canvasNodes().find((n) => n.id === 's1')!
    expect(untouched.type).toBe('script')
    expect(canvasNodes()).toHaveLength(3)
  })
})

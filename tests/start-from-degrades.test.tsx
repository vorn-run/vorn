// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render } from '@testing-library/react'
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

vi.mock('../src/renderer/components/workflow-editor/WorkflowCanvas', () => ({
  WorkflowCanvas: () => <div data-testid="canvas" />
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

// An older build, or any host that never learned the catalog methods.
;(global as unknown as { window: object }).window = {
  addEventListener: vi.fn(),
  removeEventListener: vi.fn(),
  api: {
    listWorkflowRuns: vi.fn().mockResolvedValue([]),
    isWindowMaximized: vi.fn().mockResolvedValue(false),
    onWindowMaximizedChange: vi.fn(() => () => {})
  }
}

const { WorkflowEditor } = await import('../src/renderer/components/workflow-editor/WorkflowEditor')

describe('an editor whose host cannot answer about templates', () => {
  it('still renders, because the offer is worth less than the editor', () => {
    const { getByTestId } = render(<WorkflowEditor />)
    expect(getByTestId('canvas')).toBeInTheDocument()
  })

  it('shows the start-from panel with nothing but the blank canvas in it', () => {
    const { container, getByText } = render(<WorkflowEditor />)
    expect(container.querySelector('[data-start-from]')).toBeTruthy()
    expect(getByText('Blank canvas')).toBeInTheDocument()
    expect(container.textContent).not.toContain('Templates')
  })
})

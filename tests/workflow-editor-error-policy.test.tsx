// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowNodeErrorPolicy } from '../src/shared/types'

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

// The canvas is where a node gets selected; select the one under test directly.
vi.mock('../src/renderer/components/workflow-editor/WorkflowCanvas', () => ({
  WorkflowCanvas: ({ onNodeClick }: { onNodeClick?: (id: string) => void }) => (
    <button data-testid="select-step" onClick={() => onNodeClick?.('s1')}>
      select
    </button>
  )
}))

// Stand in for the real panel so the test can drive `onErrorChange` without
// going through the select — what is under test here is what the editor does
// with the answer, not how the control renders it.
vi.mock('../src/renderer/components/workflow-editor/panels/NodeConfigPanel', () => ({
  NodeConfigPanel: ({
    onErrorChange
  }: {
    onErrorChange?: (nodeId: string, policy: WorkflowNodeErrorPolicy) => void
  }) => (
    <div>
      <button data-testid="set-continue" onClick={() => onErrorChange?.('s1', 'continue')}>
        continue
      </button>
      <button data-testid="set-stop" onClick={() => onErrorChange?.('s1', 'stop')}>
        stop
      </button>
    </div>
  )
}))

vi.mock('../src/renderer/components/workflow-editor/panels/RunHistoryPanel', () => ({
  RunHistoryPanel: () => <div />
}))

vi.mock('../src/renderer/components/workflow-editor/panels/WorkflowPropertiesPanel', () => ({
  WorkflowPropertiesPanel: () => <div />
}))

vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow: vi.fn().mockResolvedValue(undefined)
}))

const step = {
  id: 's1',
  type: 'script' as const,
  label: 'Gate',
  config: { scriptType: 'bash' as const },
  position: { x: 0, y: 120 }
}

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
    },
    step
  ],
  edges: [{ id: 'e1', source: 't', target: 's1' }],
  enabled: true,
  workspaceId: 'personal'
}

const mockState = {
  isWorkflowEditorOpen: true,
  editingWorkflowId: 'w1',
  setWorkflowEditorOpen: vi.fn(),
  setEditingWorkflowId: vi.fn(),
  addWorkflow: vi.fn(),
  updateWorkflow: vi.fn(),
  removeWorkflow: vi.fn(),
  config: { workflows: [existing], tasks: [], projects: [], defaults: {} },
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

function savedNodes(): Array<Record<string, unknown>> {
  const calls = mockState.updateWorkflow.mock.calls
  const call = calls[calls.length - 1]
  return (call?.[1] as { nodes: Array<Record<string, unknown>> }).nodes
}

function editAndSave(testId: string) {
  const { getByTestId, container } = render(<WorkflowEditor />)
  fireEvent.click(getByTestId('select-step'))
  fireEvent.click(getByTestId(testId))
  const save = Array.from(container.querySelectorAll('button')).find((b) =>
    b.textContent?.includes('Save')
  )
  if (save) fireEvent.click(save)
}

describe('the editor storing a node error policy', () => {
  beforeEach(() => {
    mockState.updateWorkflow.mockClear()
    // Each render starts from the workflow as stored, not from the last edit.
    existing.nodes[1] = { ...step }
  })

  it('records a node that opted out', () => {
    editAndSave('set-continue')
    expect(savedNodes().find((n) => n.id === 's1')).toMatchObject({ onError: 'continue' })
  })

  // `stop` is the default, so writing the word into every node would make an
  // untouched node look like it had been decided on.
  it('stores the default as absence rather than the word', () => {
    editAndSave('set-stop')
    const saved = savedNodes().find((n) => n.id === 's1')!
    expect(saved).not.toHaveProperty('onError')
  })

  it('leaves the rest of the node alone', () => {
    editAndSave('set-continue')
    expect(savedNodes().find((n) => n.id === 's1')).toMatchObject({
      label: 'Gate',
      type: 'script',
      config: { scriptType: 'bash' }
    })
  })

  it('touches only the node that changed', () => {
    editAndSave('set-continue')
    expect(savedNodes().find((n) => n.id === 't')).not.toHaveProperty('onError')
  })
})

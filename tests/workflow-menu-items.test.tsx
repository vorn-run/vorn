// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import '@testing-library/jest-dom/vitest'

const mockExecuteWorkflow = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow: (...args: unknown[]) => mockExecuteWorkflow(...args)
}))

const mockSetPending = vi.fn()
vi.mock('../src/renderer/stores', () => {
  const state = {
    setPendingContextualWorkflowId: (id: string | null) => mockSetPending(id)
  }
  return {
    useAppStore: Object.assign(
      (selector?: (s: unknown) => unknown) => (selector ? selector(state) : state),
      { getState: () => state }
    )
  }
})

import {
  buildWorkflowMenuItems,
  runWorkflowFromGlobalSurface
} from '../src/renderer/lib/workflow-menu-items'
import type { TaskConfig, TerminalSession, WorkflowDefinition } from '../src/shared/types'

function makeWorkflow(id: string, contextual: boolean): WorkflowDefinition {
  return {
    id,
    name: `wf ${id}`,
    icon: 'Zap',
    iconColor: '#fff',
    nodes: [
      {
        id: 'trigger',
        type: 'trigger',
        config: { triggerType: 'manual', contextual: contextual || undefined },
        position: { x: 0, y: 0 },
        label: 'Manual'
      }
    ],
    edges: [],
    enabled: true
  }
}

const someTask: TaskConfig = {
  id: 't',
  projectName: 'p',
  title: 'x',
  description: '',
  status: 'in_progress',
  order: 0,
  createdAt: '',
  updatedAt: ''
}
const someSession: TerminalSession = {
  id: 's',
  agentType: 'shell',
  projectName: 'p',
  projectPath: '/p',
  status: 'idle',
  createdAt: 0,
  pid: 0
}

beforeEach(() => {
  mockExecuteWorkflow.mockClear()
  mockSetPending.mockClear()
})

describe('buildWorkflowMenuItems', () => {
  it('returns only contextual workflows when called with a task context', () => {
    const items = buildWorkflowMenuItems(
      [makeWorkflow('a', true), makeWorkflow('b', false)],
      vi.fn(),
      { task: someTask }
    )
    expect(items.map((i) => i.id)).toEqual(['a'])
  })

  it('returns only contextual workflows when called with a source context', () => {
    const items = buildWorkflowMenuItems(
      [makeWorkflow('a', true), makeWorkflow('b', false)],
      vi.fn(),
      { source: someSession }
    )
    expect(items.map((i) => i.id)).toEqual(['a'])
  })

  it('returns only non-contextual workflows when called with no context', () => {
    const items = buildWorkflowMenuItems(
      [makeWorkflow('a', true), makeWorkflow('b', false)],
      vi.fn()
    )
    expect(items.map((i) => i.id)).toEqual(['b'])
  })

  it('threads context into executeWorkflow on click', () => {
    const onSelect = vi.fn()
    const items = buildWorkflowMenuItems([makeWorkflow('a', true)], onSelect, {
      source: someSession
    })
    items[0].onClick()
    expect(onSelect).toHaveBeenCalled()
    expect(mockExecuteWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'a' }),
      { task: undefined, source: someSession },
      { source: 'manual' }
    )
  })

  it('passes undefined context for non-contextual call sites', () => {
    const items = buildWorkflowMenuItems([makeWorkflow('b', false)], vi.fn())
    items[0].onClick()
    expect(mockExecuteWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'b' }),
      undefined,
      { source: 'manual' }
    )
  })
})

describe('runWorkflowFromGlobalSurface', () => {
  it('opens SourcePromptDialog for contextual workflows', () => {
    runWorkflowFromGlobalSurface(makeWorkflow('a', true))
    expect(mockSetPending).toHaveBeenCalledWith('a')
    expect(mockExecuteWorkflow).not.toHaveBeenCalled()
  })

  it('runs non-contextual workflows directly', () => {
    runWorkflowFromGlobalSurface(makeWorkflow('b', false))
    expect(mockExecuteWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'b' }),
      undefined,
      { source: 'manual' }
    )
    expect(mockSetPending).not.toHaveBeenCalled()
  })
})

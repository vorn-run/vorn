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
    setPendingWorkflowRun: (...args: unknown[]) => mockSetPending(...args)
  }
  return {
    useAppStore: Object.assign(
      (selector?: (s: unknown) => unknown) => (selector ? selector(state) : state),
      { getState: () => state }
    )
  }
})

import { buildWorkflowMenuItems, startManualRun } from '../src/renderer/lib/workflow-menu-items'
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

describe('startManualRun', () => {
  it('opens SourcePromptDialog for contextual workflows', () => {
    startManualRun(makeWorkflow('a', true))
    expect(mockSetPending).toHaveBeenCalledWith('a', undefined)
    expect(mockExecuteWorkflow).not.toHaveBeenCalled()
  })

  it('runs non-contextual workflows directly', () => {
    startManualRun(makeWorkflow('b', false))
    expect(mockExecuteWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'b' }),
      undefined,
      { source: 'manual' }
    )
    expect(mockSetPending).not.toHaveBeenCalled()
  })
})

function withInputs(wf: WorkflowDefinition): WorkflowDefinition {
  const [trigger, ...rest] = wf.nodes
  return {
    ...wf,
    nodes: [
      {
        ...trigger,
        config: {
          ...(trigger.config as Record<string, unknown>),
          triggerType: 'manual',
          inputs: [{ key: 'issue', label: 'Issue', type: 'text' }]
        } as WorkflowDefinition['nodes'][number]['config']
      },
      ...rest
    ]
  }
}

describe('workflows declaring run inputs', () => {
  it('prompts instead of launching from a global surface', () => {
    startManualRun(withInputs(makeWorkflow('wf-i', false)))
    expect(mockExecuteWorkflow).not.toHaveBeenCalled()
    expect(mockSetPending).toHaveBeenCalledWith('wf-i', undefined)
  })

  it('prompts from a card menu too, forwarding the card as context', () => {
    const items = buildWorkflowMenuItems([withInputs(makeWorkflow('wf-i', true))], () => {}, {
      task: someTask
    })
    items[0].onClick()

    expect(mockExecuteWorkflow).not.toHaveBeenCalled()
    expect(mockSetPending).toHaveBeenCalledWith('wf-i', { task: someTask, source: undefined })
  })

  it('still launches straight away when no inputs are declared', () => {
    const items = buildWorkflowMenuItems([makeWorkflow('wf-plain', true)], () => {}, {
      task: someTask
    })
    items[0].onClick()

    expect(mockSetPending).not.toHaveBeenCalled()
    expect(mockExecuteWorkflow).toHaveBeenCalledTimes(1)
  })
})

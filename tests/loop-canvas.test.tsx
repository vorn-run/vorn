// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, screen, within, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.hoisted(() => {
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
})

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => [],
  useConnectorIdFor: () => undefined,
  useConnectionIconFor: () => undefined
}))

import { WorkflowCanvas } from '../src/renderer/components/workflow-editor/WorkflowCanvas'
import type { WorkflowEdge, WorkflowNode } from '../packages/shared/src/types'

afterEach(cleanup)

const node = (
  id: string,
  type: WorkflowNode['type'],
  label: string,
  config = {}
): WorkflowNode => ({
  id,
  type,
  label,
  config: config as WorkflowNode['config'],
  position: { x: 0, y: 0 }
})

const nodes = [
  node('trigger', 'trigger', 'Manual', { triggerType: 'manual' }),
  node('fetch', 'script', 'Pull feeds', { scriptType: 'bash', scriptContent: '' }),
  node('loop', 'loop', 'Repeat until approved', {
    nodeType: 'loop',
    bodyNodeIds: ['write', 'review'],
    maxIterations: 2,
    until: { variable: '{{steps.review.approved}}', operator: 'equals', value: 'true' }
  }),
  node('write', 'script', 'Write the edition', { scriptType: 'bash', scriptContent: '' }),
  node('review', 'script', 'Review the draft', { scriptType: 'bash', scriptContent: '' }),
  node('gate', 'approval', 'Javier reviews', {})
]

const edges: WorkflowEdge[] = [
  { id: 'e1', source: 'trigger', target: 'fetch' },
  { id: 'e2', source: 'fetch', target: 'loop' },
  { id: 'e3', source: 'loop', target: 'write' },
  { id: 'e4', source: 'write', target: 'review' },
  { id: 'e5', source: 'review', target: 'gate' }
]

function renderWith(list: WorkflowNode[]) {
  return render(
    <WorkflowCanvas
      nodes={list}
      edges={edges}
      selectedNodeId={null}
      onNodeClick={() => {}}
      onInsertNode={() => {}}
      onAddParallelBranch={() => {}}
    />
  )
}

const withLoopConfig = (config: Record<string, unknown>): WorkflowNode[] =>
  nodes.map((n) => (n.id === 'loop' ? { ...n, config: config as WorkflowNode['config'] } : n))

describe('the loop rail on the canvas', () => {
  it('draws the repeated steps inside the loop, not beside it', () => {
    // The whole point of the redesign: a repeated step must not look like a
    // step that runs once.
    const { container } = renderWith(nodes)
    const rail = container.querySelector('[data-loop-rail]')
    expect(rail).not.toBeNull()
    expect(within(rail as HTMLElement).getByText('Write the edition')).toBeInTheDocument()
    expect(within(rail as HTMLElement).getByText('Review the draft')).toBeInTheDocument()
  })

  it('leaves the steps that run once outside the rail', () => {
    const { container } = renderWith(nodes)
    const rail = container.querySelector('[data-loop-rail]') as HTMLElement
    expect(within(rail).queryByText('Pull feeds')).toBeNull()
    expect(within(rail).queryByText('Javier reviews')).toBeNull()
    expect(screen.getByText('Pull feeds')).toBeInTheDocument()
    expect(screen.getByText('Javier reviews')).toBeInTheDocument()
  })

  it('draws each repeated step exactly once', () => {
    renderWith(nodes)
    expect(screen.getAllByText('Write the edition')).toHaveLength(1)
  })

  it('carries the budget in the header and the exit in the footer', () => {
    renderWith(nodes)
    expect(screen.getByText('max 2')).toBeInTheDocument()
    expect(screen.getByText(/until \{\{steps.review.approved\}\} equals true/)).toBeInTheDocument()
  })

  it('says a loop with no condition runs every pass', () => {
    renderWith(withLoopConfig({ nodeType: 'loop', bodyNodeIds: ['write'], maxIterations: 3 }))
    expect(screen.getByText(/runs every pass/)).toBeInTheDocument()
  })

  it('shows an empty loop as a drop zone rather than nothing', () => {
    renderWith(withLoopConfig({ nodeType: 'loop', bodyNodeIds: [], maxIterations: 2 }))
    expect(screen.getByText(/No steps yet/)).toBeInTheDocument()
  })
})

describe('adding a step inside the rail', () => {
  function renderWithSpy(
    list: WorkflowNode[],
    onInsertNode: (a: string, b: string | null, t: string) => void
  ) {
    return render(
      <WorkflowCanvas
        nodes={list}
        edges={edges}
        selectedNodeId={null}
        onNodeClick={() => {}}
        onInsertNode={onInsertNode as never}
        onAddParallelBranch={() => {}}
      />
    )
  }

  it('reports the loop body, so membership follows position', () => {
    // The sentinel is what tells the editor to write the edge and the
    // membership together instead of appending after the loop.
    const onInsertNode = vi.fn()
    const { container } = renderWithSpy(nodes, onInsertNode)
    const rail = container.querySelector('[data-loop-rail]') as HTMLElement

    fireEvent.click(within(rail).getByRole('button', { name: '' }))
    fireEvent.click(screen.getByText(/Launch an agent|Add an agent|agent/i))

    expect(onInsertNode).toHaveBeenCalledWith(expect.any(String), '__LOOP_BODY__', 'agent')
  })

  it('anchors the insert on the last body step', () => {
    const onInsertNode = vi.fn()
    const { container } = renderWithSpy(nodes, onInsertNode)
    const rail = container.querySelector('[data-loop-rail]') as HTMLElement

    fireEvent.click(within(rail).getByRole('button', { name: '' }))
    fireEvent.click(screen.getByText(/Launch an agent|Add an agent|agent/i))

    expect(onInsertNode.mock.calls[0][0]).toBe('review')
  })

  it('anchors on the loop itself when the body is empty', () => {
    const onInsertNode = vi.fn()
    const { container } = renderWithSpy(
      withLoopConfig({ nodeType: 'loop', bodyNodeIds: [], maxIterations: 2 }),
      onInsertNode
    )
    const rail = container.querySelector('[data-loop-rail]') as HTMLElement

    fireEvent.click(within(rail).getByRole('button', { name: '' }))
    fireEvent.click(screen.getByText(/Launch an agent|Add an agent|agent/i))

    expect(onInsertNode.mock.calls[0][0]).toBe('loop')
  })
})

describe('where a loop can be added', () => {
  it('is offered on the trunk', () => {
    const { container } = renderWith(nodes)
    // The + between trunk steps, outside the rail.
    const buttons = container.querySelectorAll('button')
    fireEvent.click(buttons[buttons.length - 1])
    expect(screen.queryByText(/Repeat steps/)).not.toBeNull()
  })
})

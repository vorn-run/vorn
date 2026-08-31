// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, screen, within, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.hoisted(() => {
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
  // The graph surface measures itself with ResizeObserver, which jsdom lacks.
  // Nodes carry explicit dimensions, so a no-op observer is enough to render.
  class NoopResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(window, 'ResizeObserver', {
    value: NoopResizeObserver,
    writable: true
  })
  ;(globalThis as { ResizeObserver?: unknown }).ResizeObserver = NoopResizeObserver
})

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => [],
  useConnectorIdFor: () => undefined,
  useConnectionIconFor: () => undefined
}))

import { WorkflowCanvas } from '../src/renderer/components/workflow-editor/WorkflowCanvas'
import { NODE_SELECTED } from '../src/renderer/components/workflow-editor/node-visuals'
import { WORKFLOW_STATUS_DOT } from '../src/renderer/lib/workflow-status'
import type { NodeExecutionStatus } from '../src/shared/types'
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

function renderWith(
  list: WorkflowNode[],
  selectedNodeId: string | null = null,
  edgeList: WorkflowEdge[] = edges,
  nodeStatus?: Record<string, NodeExecutionStatus>
) {
  return render(
    <WorkflowCanvas
      nodes={list}
      edges={edgeList}
      nodeStatus={nodeStatus}
      selectedNodeId={selectedNodeId}
      onNodeClick={() => {}}
      onOpenLibrary={() => {}}
      libraryAnchor={null}
      onConnectEdge={() => {}}
      onPositionsCommit={() => {}}
      onTidyUp={() => {}}
    />
  )
}

const forkEdges: WorkflowEdge[] = [
  { id: 'f1', source: 'trigger', target: 'cond' },
  { id: 'f2', source: 'cond', target: 'yes', conditionBranch: 'true' },
  { id: 'f3', source: 'cond', target: 'no', conditionBranch: 'false' }
]

const forkNodes: WorkflowNode[] = [
  node('trigger', 'trigger', 'Manual', { triggerType: 'manual' }),
  node('cond', 'condition', 'Ready?', { variable: 'status', operator: 'equals', value: 'ok' }),
  node('yes', 'script', 'Ship it', { scriptType: 'bash', scriptContent: '' }),
  node('no', 'script', 'Hold', { scriptType: 'bash', scriptContent: '' })
]

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
  function renderWithSpy(list: WorkflowNode[], onOpenLibrary: (a: unknown) => void) {
    return render(
      <WorkflowCanvas
        nodes={list}
        edges={edges}
        selectedNodeId={null}
        onNodeClick={() => {}}
        onOpenLibrary={onOpenLibrary as never}
        libraryAnchor={null}
        onConnectEdge={() => {}}
        onPositionsCommit={() => {}}
        onTidyUp={() => {}}
      />
    )
  }

  it('opens the library scoped to the body, so membership follows position', () => {
    // The sentinel is what tells the editor to write the edge and the
    // membership together instead of appending after the loop.
    const onOpenLibrary = vi.fn()
    const { container } = renderWithSpy(nodes, onOpenLibrary)
    const rail = container.querySelector('[data-loop-rail]') as HTMLElement

    fireEvent.click(within(rail).getByRole('button', { name: 'Add a step' }))

    expect(onOpenLibrary).toHaveBeenCalledWith(
      expect.objectContaining({ beforeNodeId: '__LOOP_BODY__', bodyOnly: true })
    )
  })

  it('anchors the insert on the last body step', () => {
    const onOpenLibrary = vi.fn()
    const { container } = renderWithSpy(nodes, onOpenLibrary)
    const rail = container.querySelector('[data-loop-rail]') as HTMLElement

    fireEvent.click(within(rail).getByRole('button', { name: 'Add a step' }))

    expect(onOpenLibrary.mock.calls[0][0].afterNodeId).toBe('review')
  })

  it('anchors on the loop itself when the body is empty', () => {
    const onOpenLibrary = vi.fn()
    const { container } = renderWithSpy(
      withLoopConfig({ nodeType: 'loop', bodyNodeIds: [], maxIterations: 2 }),
      onOpenLibrary
    )
    const rail = container.querySelector('[data-loop-rail]') as HTMLElement

    fireEvent.click(within(rail).getByRole('button', { name: 'Add a step' }))

    expect(onOpenLibrary.mock.calls[0][0].afterNodeId).toBe('loop')
  })
})

describe('where a loop can be added', () => {
  it('a trunk anchor reaches the library unrestricted', () => {
    const onOpenLibrary = vi.fn()
    const { container } = render(
      <WorkflowCanvas
        nodes={nodes}
        edges={edges}
        selectedNodeId={null}
        onNodeClick={() => {}}
        onOpenLibrary={onOpenLibrary as never}
        libraryAnchor={null}
        onConnectEdge={() => {}}
        onPositionsCommit={() => {}}
        onTidyUp={() => {}}
      />
    )
    const rail = container.querySelector('[data-loop-rail]') as HTMLElement
    const adds = screen
      .getAllByRole('button', { name: 'Add a step' })
      .filter((b) => !rail.contains(b))
    fireEvent.click(adds[adds.length - 1])
    expect(onOpenLibrary).toHaveBeenCalledWith(
      expect.objectContaining({ insideBranch: false, bodyOnly: false })
    )
  })
})

describe('the canvas reads in one colour', () => {
  it('marks a selected loop the same way a selected step is marked', () => {
    // The enclosure carried its own copy of the selection string and had to be
    // kept in step with eight cards by hand.
    const { container } = renderWith(nodes, 'loop')
    const rail = container.querySelector('[data-loop-rail]') as HTMLElement
    expect(rail.className).toContain(NODE_SELECTED)
    expect(rail.className).not.toContain('bronzo')
  })

  it('gives both sides of a fork the same label treatment', () => {
    // True was green and False red, which said one path is the good one and
    // the other a failure. A condition is a fork; the word says which way.
    const { container } = renderWith(forkNodes, null, forkEdges)
    const labels = [...container.querySelectorAll('[data-branch-label]')]
    expect(labels).toHaveLength(2)
    expect(labels[0].className).toBe(labels[1].className)
    for (const el of labels) {
      expect(el.className).not.toMatch(/green|red|bronzo|danger/)
    }
  })
})

describe('a live run on the canvas', () => {
  it('shows each node the state it is in', () => {
    // The canvas has always accepted a per-node status and nothing ever passed
    // one, so this dot never appeared outside its own tests: a run could park on
    // a gate in the runs list while its node here looked idle.
    const { container } = renderWith(nodes, null, edges, {
      fetch: 'running',
      write: 'success'
    })
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.running}`)).toBeInTheDocument()
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.success}`)).toBeInTheDocument()
  })

  it('accents a node the run is waiting on', () => {
    const { container } = renderWith(nodes, null, edges, { gate: 'waiting' })
    expect(container.querySelector(`.${WORKFLOW_STATUS_DOT.waiting}`)).toBeInTheDocument()
  })

  it('says nothing when nothing is running', () => {
    const { container } = renderWith(nodes)
    for (const s of ['running', 'waiting', 'success', 'error'] as const) {
      expect(container.querySelector(`.${WORKFLOW_STATUS_DOT[s]}`)).toBeNull()
    }
  })

  it('shows the loop itself the state it is in', () => {
    // The rail draws its own header rather than going through a node card, so
    // it is the one node that has to read its status separately — and a loop
    // does have one: running while it iterates, error when its body is empty or
    // holds a gate. Without this the rail sat plain while the run was inside it.
    const { container } = renderWith(nodes, null, edges, { loop: 'running' })
    const dot = container.querySelector('[data-loop-status]')
    expect(dot).toBeInTheDocument()
    expect(dot?.className).toContain(WORKFLOW_STATUS_DOT.running)
  })

  it('leaves the rail plain when the loop is not part of a live run', () => {
    const { container } = renderWith(nodes, null, edges, { fetch: 'running' })
    expect(container.querySelector('[data-loop-status]')).toBeNull()
  })
})

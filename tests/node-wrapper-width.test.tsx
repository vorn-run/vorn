// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.hoisted(() => {
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
  class NoopResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(window, 'ResizeObserver', { value: NoopResizeObserver, writable: true })
  ;(globalThis as { ResizeObserver?: unknown }).ResizeObserver = NoopResizeObserver
})

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => [],
  useConnectorIdFor: () => 'github',
  useConnectionIconFor: () => undefined
}))

import { WorkflowCanvas } from '../src/renderer/components/workflow-editor/WorkflowCanvas'
import type { WorkflowEdge, WorkflowNode } from '../packages/shared/src/types'

afterEach(cleanup)

const trigger: WorkflowNode = {
  id: 't',
  type: 'trigger',
  label: 'Manual',
  config: { triggerType: 'manual' },
  position: { x: 0, y: 0 }
}

/** The step from the report: one target handle more than the trigger above it. */
const connectorAction: WorkflowNode = {
  id: 'a',
  type: 'callConnectorAction',
  label: 'Connector Action',
  slug: 'connector-action',
  config: { connectionId: 'conn-1', action: 'echo' } as WorkflowNode['config'],
  position: { x: 0, y: 120 }
}

const edges: WorkflowEdge[] = [{ id: 'e1', source: 't', target: 'a' }]

function renderCanvas(nodes: WorkflowNode[]) {
  return render(
    <WorkflowCanvas
      nodes={nodes}
      edges={edges}
      selectedNodeId={null}
      libraryAnchor={null}
      onNodeClick={vi.fn()}
      onOpenLibrary={vi.fn()}
      onConnectEdge={vi.fn()}
      onPositionsCommit={vi.fn()}
      onTidyUp={vi.fn()}
      // Given so the hover toolbar renders and is checked for staying out of flow.
      onDeleteNode={vi.fn()}
      onRunToStep={vi.fn()}
    />
  )
}

/** The wrapper each step draws inside, which is what carries the ports. */
function wrappers(container: HTMLElement): HTMLElement[] {
  return [...container.querySelectorAll('.react-flow__node')].map(
    (n) => n.firstElementChild as HTMLElement
  )
}

describe('what a step node occupies', () => {
  // A wrapper that took its width from its content would be one handle wider
  // for a step than for a trigger, and both the card and the port centred on
  // it would sit half that difference to the right.
  it('gives a trigger and a connector action the same stated width', () => {
    const { container } = renderCanvas([trigger, connectorAction])
    const [first, second] = wrappers(container)

    expect(first.className).toContain('w-[280px]')
    expect(second.className).toContain('w-[280px]')
  })

  it('centres every port on that width, however many handles a node has', () => {
    const { container } = renderCanvas([trigger, connectorAction])
    const [first, second] = wrappers(container)

    // The trigger sends only; the step below it also receives.
    expect(first.querySelectorAll('.react-flow__handle')).toHaveLength(1)
    expect(second.querySelectorAll('.react-flow__handle')).toHaveLength(2)
    // Both sit on the same declared box, so both centre on the same x.
    for (const handle of container.querySelectorAll('.react-flow__handle')) {
      expect(handle.className).toMatch(/react-flow__handle-(top|bottom)/)
    }
  })

  it('keeps every sibling of the card out of the flow that sets the width', () => {
    const { container } = renderCanvas([trigger, connectorAction])

    for (const wrapper of wrappers(container)) {
      for (const child of wrapper.children) {
        const isCard = child.className.includes('w-[280px]')
        // Anything that is not the card must be positioned, or it would widen
        // the wrapper and take the card off the column with it.
        if (!isCard) expect(child.className).toMatch(/absolute/)
      }
    }
  })
})

// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, screen, fireEvent, within } from '@testing-library/react'
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
  useConnections: () => [{ id: 'conn1', name: 'GitHub' }],
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
  config: Record<string, unknown> = {}
): WorkflowNode => ({
  id,
  type,
  label,
  config: config as WorkflowNode['config'],
  position: { x: 0, y: 0 }
})

const nodes = [
  node('t', 'trigger', 'Manual', { triggerType: 'manual' }),
  node('a', 'script', 'First step', { scriptType: 'bash', scriptContent: '' }),
  node('b', 'script', 'Second step', { scriptType: 'bash', scriptContent: '' })
]
const edges: WorkflowEdge[] = [
  { id: 'e1', source: 't', target: 'a' },
  { id: 'e2', source: 'a', target: 'b' }
]

function renderCanvas(over: Partial<Parameters<typeof WorkflowCanvas>[0]> = {}) {
  const handlers = {
    onNodeClick: vi.fn(),
    onInsertNode: vi.fn(),
    onAddParallelBranch: vi.fn(),
    onConnectEdge: vi.fn(),
    onPaletteInsert: vi.fn(),
    onPositionsCommit: vi.fn(),
    onDeleteNode: vi.fn(),
    onTidyUp: vi.fn()
  }
  const utils = render(
    <WorkflowCanvas nodes={nodes} edges={edges} selectedNodeId={null} {...handlers} {...over} />
  )
  return { ...utils, ...handlers }
}

describe('inserting on an edge', () => {
  it('offers the + while the edge is hovered and splices with the real endpoints', () => {
    const { container, onInsertNode } = renderCanvas()
    const edge = container.querySelector('.react-flow__edge') as SVGGElement
    fireEvent.mouseEnter(edge.firstChild as Element)
    const add = screen
      .getAllByRole('button', { name: 'Add a step' })
      .find((b) => b.closest('.react-flow__edgelabel-renderer'))!
    fireEvent.click(add)
    fireEvent.click(screen.getByText('Add a script'))
    expect(onInsertNode).toHaveBeenCalledWith('t', 'a', 'script')
  })
})

describe('the hover toolbar', () => {
  it('deletes the hovered step, and never offers itself on the trigger', () => {
    const { container, onDeleteNode } = renderCanvas()
    const deletes = screen.getAllByRole('button', { name: 'Delete step' })
    // One per non-trigger node: the trigger card carries none.
    expect(deletes).toHaveLength(2)
    fireEvent.click(deletes[0])
    expect(onDeleteNode).toHaveBeenCalledTimes(1)
    expect(container.querySelectorAll('.react-flow__node').length).toBeGreaterThan(2)
  })
})

describe('the keyboard on the canvas', () => {
  it('deletes the selected step with Backspace but spares the trigger', () => {
    const { container, onDeleteNode } = renderCanvas({ selectedNodeId: 'b' })
    const wrapper = container.querySelector('[tabindex="0"]') as HTMLElement
    fireEvent.keyDown(wrapper, { key: 'Backspace' })
    expect(onDeleteNode).toHaveBeenCalledWith('b')
  })

  it('never deletes the trigger', () => {
    const { container, onDeleteNode } = renderCanvas({ selectedNodeId: 't' })
    const wrapper = container.querySelector('[tabindex="0"]') as HTMLElement
    fireEvent.keyDown(wrapper, { key: 'Delete' })
    expect(onDeleteNode).not.toHaveBeenCalled()
  })

  it('opens the node search on Tab, anchored to the leaf', async () => {
    const { container } = renderCanvas()
    const wrapper = container.querySelector('[tabindex="0"]') as HTMLElement
    wrapper.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: 800, height: 600, right: 800, bottom: 600, x: 0, y: 0 }) as DOMRect
    fireEvent.keyDown(wrapper, { key: 'Tab' })
    expect(await screen.findByPlaceholderText('Search steps and actions')).toBeInTheDocument()
  })
})

describe('picking from the palette', () => {
  it('hands the pick and its anchor to the editor', async () => {
    const { container, onPaletteInsert } = renderCanvas()
    const wrapper = container.querySelector('[tabindex="0"]') as HTMLElement
    wrapper.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: 800, height: 600, right: 800, bottom: 600, x: 0, y: 0 }) as DOMRect
    fireEvent.keyDown(wrapper, { key: 'Tab' })
    const palette = (await screen.findByPlaceholderText('Search steps and actions')).closest(
      '[data-node-palette]'
    ) as HTMLElement
    fireEvent.click(within(palette).getByText('Add an agent'))
    expect(onPaletteInsert).toHaveBeenCalledWith(
      { kind: 'type', type: 'agent' },
      'b',
      expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) })
    )
  })
})

describe('the tidy up control', () => {
  it('sits with the zoom controls', () => {
    const { onTidyUp } = renderCanvas()
    fireEvent.click(screen.getByRole('button', { name: 'Tidy up' }))
    expect(onTidyUp).toHaveBeenCalled()
  })
})

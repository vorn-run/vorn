// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, screen, fireEvent } from '@testing-library/react'
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
  useConnectorIdFor: () => undefined,
  useConnectionIconFor: () => undefined
}))

import {
  WorkflowCanvas,
  InsertAnchor
} from '../src/renderer/components/workflow-editor/WorkflowCanvas'
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
    onOpenLibrary: vi.fn(),
    onConnectEdge: vi.fn(),
    onPositionsCommit: vi.fn(),
    onDeleteNode: vi.fn(),
    onTidyUp: vi.fn()
  }
  const utils = render(
    <WorkflowCanvas
      nodes={nodes}
      edges={edges}
      selectedNodeId={null}
      libraryAnchor={null}
      {...handlers}
      {...over}
    />
  )
  return { ...utils, ...handlers }
}

describe('every + opens the library at its anchor', () => {
  it('a hovered edge + reports the edge endpoints', () => {
    const { container, onOpenLibrary } = renderCanvas()
    const edge = container.querySelector('.react-flow__edge') as SVGGElement
    fireEvent.mouseEnter(edge.firstChild as Element)
    const add = screen
      .getAllByRole('button', { name: 'Add a step' })
      .find((b) => b.closest('.react-flow__edgelabel-renderer'))!
    fireEvent.click(add)
    expect(onOpenLibrary).toHaveBeenCalledWith({
      afterNodeId: 't',
      beforeNodeId: 'a',
      insideBranch: false,
      bodyOnly: false
    } satisfies InsertAnchor)
  })

  it('the trailing + reports an append anchor', () => {
    const { onOpenLibrary } = renderCanvas()
    const adds = screen.getAllByRole('button', { name: 'Add a step' })
    fireEvent.click(adds[adds.length - 1])
    expect(onOpenLibrary).toHaveBeenCalledWith({
      afterNodeId: 'b',
      beforeNodeId: null,
      insideBranch: false,
      bodyOnly: false
    } satisfies InsertAnchor)
  })

  it('Tab anchors on the leaf', () => {
    const { container, onOpenLibrary } = renderCanvas()
    const wrapper = container.querySelector('[tabindex="0"]') as HTMLElement
    fireEvent.keyDown(wrapper, { key: 'Tab' })
    expect(onOpenLibrary).toHaveBeenCalledWith(
      expect.objectContaining({ afterNodeId: 'b', beforeNodeId: null })
    )
  })

  it('the anchored + stays lit', () => {
    renderCanvas({
      libraryAnchor: { afterNodeId: 'b', beforeNodeId: null, insideBranch: false, bodyOnly: false }
    })
    const adds = screen.getAllByRole('button', { name: 'Add a step' })
    expect(adds[adds.length - 1].className).toContain('border-white/40')
  })
})

describe('the hover toolbar', () => {
  it('offers delete on every node, the trigger included', () => {
    const { onDeleteNode } = renderCanvas()
    const deletes = screen.getAllByRole('button', { name: 'Delete step' })
    expect(deletes).toHaveLength(3)
    fireEvent.click(deletes[0])
    expect(onDeleteNode).toHaveBeenCalledTimes(1)
  })

  it('offers replace only on swappable steps, never the trigger', () => {
    const { onOpenLibrary } = renderCanvas()
    const replaces = screen.getAllByRole('button', { name: 'Replace step' })
    // The two scripts are swappable; the trigger has its own library path.
    expect(replaces).toHaveLength(2)
    fireEvent.click(replaces[0])
    expect(onOpenLibrary).toHaveBeenCalledWith(
      expect.objectContaining({ replaceNodeId: 'a', afterNodeId: 'a' })
    )
  })
})

describe('the keyboard on the canvas', () => {
  it('deletes the selected step with Backspace', () => {
    const { container, onDeleteNode } = renderCanvas({ selectedNodeId: 'b' })
    const wrapper = container.querySelector('[tabindex="0"]') as HTMLElement
    fireEvent.keyDown(wrapper, { key: 'Backspace' })
    expect(onDeleteNode).toHaveBeenCalledWith('b')
  })

  it('deletes the selected trigger too', () => {
    const { container, onDeleteNode } = renderCanvas({ selectedNodeId: 't' })
    const wrapper = container.querySelector('[tabindex="0"]') as HTMLElement
    fireEvent.keyDown(wrapper, { key: 'Delete' })
    expect(onDeleteNode).toHaveBeenCalledWith('t')
  })
})

describe('the tidy up control', () => {
  it('sits with the zoom controls', () => {
    const { onTidyUp } = renderCanvas()
    fireEvent.click(screen.getByRole('button', { name: 'Tidy up' }))
    expect(onTidyUp).toHaveBeenCalled()
  })
})

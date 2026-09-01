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
  useConnectorIdFor: () => undefined,
  useConnectionIconFor: () => undefined
}))

const internalsCalls = vi.hoisted(() => ({ ids: [] as string[] }))
vi.mock('@xyflow/react', async () => {
  const actual = await vi.importActual<typeof import('@xyflow/react')>('@xyflow/react')
  return {
    ...actual,
    useUpdateNodeInternals: () => {
      const real = actual.useUpdateNodeInternals()
      return (id: string | string[]) => {
        for (const one of Array.isArray(id) ? id : [id]) internalsCalls.ids.push(one)
        return real(id)
      }
    }
  }
})

import { WorkflowCanvas } from '../src/renderer/components/workflow-editor/WorkflowCanvas'
import { toCanvasElements } from '../src/renderer/lib/workflow-canvas-layout'
import type { WorkflowEdge, WorkflowNode } from '../packages/shared/src/types'

afterEach(() => {
  cleanup()
  internalsCalls.ids.length = 0
})

const trigger: WorkflowNode = {
  id: 't',
  type: 'trigger',
  label: 'Manual',
  config: { triggerType: 'manual' },
  position: { x: 0, y: 0 }
}

const script: WorkflowNode = {
  id: 'a',
  type: 'script',
  label: 'Old script',
  slug: 'old-script',
  // A script with content renders a preview, so its estimated height is the tall one.
  config: { scriptType: 'bash', scriptContent: 'echo hi' } as WorkflowNode['config'],
  position: { x: 0, y: 120 }
}

const swapped: WorkflowNode = {
  id: 'a',
  type: 'approval',
  label: 'Approval gate',
  slug: 'approval-gate',
  config: {} as WorkflowNode['config'],
  position: { x: 0, y: 120 }
}

const edges: WorkflowEdge[] = [{ id: 'e1', source: 't', target: 'a' }]

const handlers = {
  onNodeClick: vi.fn(),
  onOpenLibrary: vi.fn(),
  onConnectEdge: vi.fn(),
  onPositionsCommit: vi.fn(),
  onTidyUp: vi.fn()
}

function renderCanvas(nodes: WorkflowNode[]) {
  return render(
    <WorkflowCanvas
      nodes={nodes}
      edges={edges}
      selectedNodeId={null}
      libraryAnchor={null}
      {...handlers}
    />
  )
}

const manualTrigger: WorkflowNode = {
  id: 't',
  type: 'trigger',
  label: 'Manual Trigger',
  config: { triggerType: 'manual' },
  position: { x: 0, y: 0 }
}

const recurringTrigger: WorkflowNode = {
  id: 't',
  type: 'trigger',
  label: 'Schedule (Recurring)',
  config: { triggerType: 'recurring', cron: '0 9 * * *' } as WorkflowNode['config'],
  position: { x: 0, y: 0 }
}

describe('re-measuring after a replace-in-place', () => {
  it('re-measures a trigger whose kind swaps under the same id', () => {
    const { rerender } = renderCanvas([manualTrigger, script])
    expect(internalsCalls.ids).toHaveLength(0)

    rerender(
      <WorkflowCanvas
        nodes={[recurringTrigger, script]}
        edges={edges}
        selectedNodeId={null}
        libraryAnchor={null}
        {...handlers}
      />
    )
    expect(internalsCalls.ids).toContain('t')
  })

  it('re-declares the node height and source-handle y for the swapped type', () => {
    const handleY = (nodes: WorkflowNode[]) => {
      const rf = toCanvasElements(nodes, edges).nodes.find((n) => n.id === 'a')!
      const source = (rf.handles ?? []).find((h) => h.type === 'source')!
      return { height: rf.initialHeight, y: source.y }
    }
    const before = handleY([trigger, script])
    const after = handleY([trigger, swapped])
    // The script previews its content (tall card); the bare approval does not.
    expect(before.height).not.toBe(after.height)
    expect(before.y).not.toBe(after.y)
    expect(after.y).toBe(after.height)
  })

  it("leaves the initial mount to React Flow's own measure", () => {
    renderCanvas([trigger, script])
    expect(internalsCalls.ids).toHaveLength(0)
  })

  it('asks React Flow to re-measure the swapped node', () => {
    const { rerender } = renderCanvas([trigger, script])
    expect(internalsCalls.ids).toHaveLength(0)

    rerender(
      <WorkflowCanvas
        nodes={[trigger, swapped]}
        edges={edges}
        selectedNodeId={null}
        libraryAnchor={null}
        {...handlers}
      />
    )
    expect(internalsCalls.ids).toContain('a')
  })
})

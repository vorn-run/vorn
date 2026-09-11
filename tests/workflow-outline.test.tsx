// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { flowOrder } from '../src/renderer/lib/workflow-helpers'
import { openingViewport } from '../src/renderer/lib/workflow-canvas-layout'
import { StepOutline } from '../src/renderer/components/workflow-editor/StepOutline'
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

describe('the order the outline lists steps in', () => {
  it('follows the chain from the trigger', () => {
    const nodes = [
      node('t', 'trigger', 'Manual', { triggerType: 'manual' }),
      node('a', 'script', 'Build'),
      node('b', 'script', 'Ship')
    ]
    const edges: WorkflowEdge[] = [
      { id: 'e1', source: 't', target: 'a' },
      { id: 'e2', source: 'a', target: 'b' }
    ]
    expect(flowOrder(nodes, edges).map((s) => [s.node.id, s.depth])).toEqual([
      ['t', 0],
      ['a', 0],
      ['b', 0]
    ])
  })

  it('puts a loop body one level in, drawn inside its loop', () => {
    const nodes = [
      node('t', 'trigger', 'Manual', { triggerType: 'manual' }),
      node('loop', 'loop', 'Until it is clean', {
        nodeType: 'loop',
        bodyNodeIds: ['w', 'r'],
        maxIterations: 2
      }),
      node('w', 'script', 'Check'),
      node('r', 'script', 'Review'),
      node('after', 'script', 'Push the branch')
    ]
    const edges: WorkflowEdge[] = [
      { id: 'e1', source: 't', target: 'loop' },
      { id: 'e2', source: 'loop', target: 'w' },
      { id: 'e3', source: 'w', target: 'r' },
      { id: 'e4', source: 'r', target: 'after' }
    ]
    expect(flowOrder(nodes, edges).map((s) => [s.node.id, s.depth, s.within])).toEqual([
      ['t', 0, undefined],
      ['loop', 0, undefined],
      ['w', 1, 'loop'],
      ['r', 1, 'loop'],
      ['after', 0, undefined]
    ])
  })

  it('lists both branches of a condition one level in, then the step they join at', () => {
    const nodes = [
      node('t', 'trigger', 'Manual', { triggerType: 'manual' }),
      node('c', 'condition', 'Is it a blocker?', { variable: 'x', operator: 'equals', value: '1' }),
      node('yes', 'script', 'Say it is urgent'),
      node('no', 'script', 'File the triage'),
      node('join', 'script', 'Close out')
    ]
    const edges: WorkflowEdge[] = [
      { id: 'e1', source: 't', target: 'c' },
      { id: 'e2', source: 'c', target: 'yes', conditionBranch: 'true' },
      { id: 'e3', source: 'c', target: 'no', conditionBranch: 'false' },
      { id: 'e4', source: 'yes', target: 'join' },
      { id: 'e5', source: 'no', target: 'join' }
    ]
    const order = flowOrder(nodes, edges)
    expect(order.map((s) => s.node.id)).toEqual(['t', 'c', 'yes', 'no', 'join'])
    expect(order.find((s) => s.node.id === 'yes')?.depth).toBe(1)
    expect(order.find((s) => s.node.id === 'join')?.depth).toBe(0)
  })

  it('keeps a step nothing connects to, at the end', () => {
    const nodes = [node('t', 'trigger', 'Manual'), node('stray', 'script', 'Forgotten')]
    expect(flowOrder(nodes, []).map((s) => s.node.id)).toEqual(['t', 'stray'])
  })
})

describe('where a workflow opens', () => {
  const placed = [
    { type: 'step', position: { x: -140, y: 0 } },
    { type: 'step', position: { x: -140, y: 114 } },
    { type: 'addStep', position: { x: -12, y: 230 } }
  ]

  it('is 100%, with the steps centred and the first near the top', () => {
    expect(openingViewport(placed, 800)).toEqual({ x: 400, y: 48, zoom: 1 })
  })

  it('centres the add button of an empty workflow', () => {
    expect(
      openingViewport([{ type: 'addTrigger', position: { x: -20, y: 0 }, width: 40 }], 600)
    ).toEqual({ x: 300, y: 48, zoom: 1 })
  })
})

describe('the step outline', () => {
  const steps = [
    { node: node('t', 'trigger', 'Manual'), depth: 0 },
    { node: node('loop', 'loop', 'Until it is clean'), depth: 0 },
    { node: node('r', 'launchAgent', 'Review'), depth: 1, within: 'loop' }
  ]

  it('lists every step, with a count', () => {
    render(<StepOutline steps={steps} focusedId={null} visibleIds={new Set()} onFocus={vi.fn()} />)
    expect(screen.getByText('Steps · 3')).toBeInTheDocument()
    expect(screen.getAllByRole('button')).toHaveLength(3)
  })

  it('marks the step the keys are on, and fades the ones off screen', () => {
    render(
      <StepOutline steps={steps} focusedId="r" visibleIds={new Set(['t'])} onFocus={vi.fn()} />
    )
    expect(screen.getByRole('button', { name: 'Review' })).toHaveAttribute('aria-current', 'step')
    expect(screen.getByRole('button', { name: 'Manual' }).className).not.toContain('text-ink-faint')
    expect(screen.getByRole('button', { name: 'Until it is clean' }).className).toContain(
      'text-ink-faint'
    )
  })

  it('asks the canvas to show a step when it is clicked', () => {
    const onFocus = vi.fn()
    render(<StepOutline steps={steps} focusedId={null} visibleIds={new Set()} onFocus={onFocus} />)
    fireEvent.click(screen.getByRole('button', { name: 'Review' }))
    expect(onFocus).toHaveBeenCalledWith('r')
  })
})

import { describe, it, expect } from 'vitest'
import { loopShouldStop, MAX_LOOP_ITERATIONS } from '../src/renderer/lib/workflow-execution'
import { validateLoopBodies } from '../packages/mcp/src/tools/workflows'
import { nodesAfter } from '../src/renderer/lib/workflow-helpers'
import type { ConditionConfig, WorkflowEdge, WorkflowNode } from '../packages/shared/src/types'

const approved: ConditionConfig = {
  variable: '{{steps.review.approved}}',
  operator: 'equals',
  value: 'true'
}

describe('loopShouldStop', () => {
  it('never stops early when no condition is declared', () => {
    // Such a loop is "run the body exactly maxIterations times".
    expect(loopShouldStop(undefined, 'anything', 'true')).toBe(false)
  })

  it('stops once the condition holds', () => {
    expect(loopShouldStop(approved, 'true', 'true')).toBe(true)
  })

  it('keeps going while it does not', () => {
    expect(loopShouldStop(approved, 'false', 'true')).toBe(false)
  })

  it('keeps going when the variable did not resolve', () => {
    // An unresolved template yields '' — treating that as "approved" would end
    // the loop on a typo, which is the worst possible reading of silence.
    expect(loopShouldStop(approved, '', 'true')).toBe(false)
  })

  it('supports isNotEmpty for a step that either produced something or did not', () => {
    const cfg: ConditionConfig = {
      variable: '{{steps.review.blocking}}',
      operator: 'isEmpty',
      value: ''
    }
    expect(loopShouldStop(cfg, '', '')).toBe(true)
    expect(loopShouldStop(cfg, 'one problem', '')).toBe(false)
  })
})

describe('loop bounds', () => {
  // The cap is the contract, not a safety net: an LLM judge asked "is this good
  // yet" trends toward yes, so the bound is what actually ends the loop.
  const clamp = (requested: number): number =>
    Math.min(Math.max(1, Math.floor(requested)), MAX_LOOP_ITERATIONS)

  it('never runs zero passes', () => {
    expect(clamp(0)).toBe(1)
    expect(clamp(-5)).toBe(1)
  })

  it('caps a runaway request', () => {
    expect(clamp(1000)).toBe(MAX_LOOP_ITERATIONS)
  })

  it('passes a sensible request through', () => {
    expect(clamp(2)).toBe(2)
  })

  it('floors a fractional request rather than looping forever on NaN arithmetic', () => {
    expect(clamp(2.9)).toBe(2)
  })
})

describe('validateLoopBodies', () => {
  const loop = (bodyNodeIds: string[]): Parameters<typeof validateLoopBodies>[0][number] => ({
    id: 'loop-1',
    type: 'loop',
    label: 'Revise until approved',
    config: { bodyNodeIds, maxIterations: 2 }
  })
  const step = (id: string): Parameters<typeof validateLoopBodies>[0][number] => ({
    id,
    type: 'script',
    label: id,
    config: {}
  })

  it('accepts a body of steps that exist', () => {
    expect(validateLoopBodies([loop(['write', 'review']), step('write'), step('review')])).toEqual(
      []
    )
  })

  it('rejects a body step that does not exist', () => {
    // Authoring-time, because at run time it is a silently shorter loop.
    const errors = validateLoopBodies([loop(['write', 'typo']), step('write')])
    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('unknown body step "typo"')
  })

  it('rejects a loop that contains itself', () => {
    const errors = validateLoopBodies([loop(['loop-1']), step('write')])
    expect(errors.some((e) => e.includes('lists itself'))).toBe(true)
  })

  it('ignores workflows with no loop', () => {
    expect(validateLoopBodies([step('a'), step('b')])).toEqual([])
  })
})

describe('nodesAfter (which steps a loop can repeat)', () => {
  const n = (id: string, type: WorkflowNode['type'] = 'script'): WorkflowNode => ({
    id,
    type,
    label: id,
    config: {} as WorkflowNode['config'],
    position: { x: 0, y: 0 }
  })

  const nodes = [n('trigger', 'trigger'), n('loop', 'loop'), n('write'), n('review'), n('publish')]
  const edges: WorkflowEdge[] = [
    { id: 'e1', source: 'trigger', target: 'loop' },
    { id: 'e2', source: 'loop', target: 'write' },
    { id: 'e3', source: 'write', target: 'review' },
    { id: 'e4', source: 'review', target: 'publish' }
  ]

  it('offers every step downstream of the loop', () => {
    expect(nodesAfter(nodes, edges, 'loop').map((x) => x.id)).toEqual([
      'write',
      'review',
      'publish'
    ])
  })

  it('never offers an upstream step, which the loop’s own inputs may depend on', () => {
    expect(nodesAfter(nodes, edges, 'loop').map((x) => x.id)).not.toContain('trigger')
  })

  it('never offers the loop itself', () => {
    expect(nodesAfter(nodes, edges, 'loop').map((x) => x.id)).not.toContain('loop')
  })

  it('terminates on a cyclic graph', () => {
    const cyclic: WorkflowEdge[] = [...edges, { id: 'e5', source: 'publish', target: 'write' }]
    expect(nodesAfter(nodes, cyclic, 'loop').length).toBe(3)
  })
})

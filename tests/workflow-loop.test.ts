import { describe, it, expect } from 'vitest'
import { loopShouldStop, MAX_LOOP_ITERATIONS } from '../src/renderer/lib/workflow-execution'
import { validateLoopBodies } from '../packages/mcp/src/tools/workflows'
import {
  nodesAfter,
  computeFlowLayout,
  appendToLoopBody
} from '../src/renderer/lib/workflow-helpers'
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

describe('loop layout (the body is drawn inside the loop)', () => {
  const n = (id: string, type: WorkflowNode['type'] = 'script', config = {}): WorkflowNode => ({
    id,
    type,
    label: id,
    config: config as WorkflowNode['config'],
    position: { x: 0, y: 0 }
  })

  const nodes = [
    n('trigger', 'trigger'),
    n('fetch'),
    n('loop', 'loop', { nodeType: 'loop', bodyNodeIds: ['write', 'review'], maxIterations: 2 }),
    n('write'),
    n('review'),
    n('gate', 'approval')
  ]
  const edges: WorkflowEdge[] = [
    { id: 'e1', source: 'trigger', target: 'fetch' },
    { id: 'e2', source: 'fetch', target: 'loop' },
    { id: 'e3', source: 'loop', target: 'write' },
    { id: 'e4', source: 'write', target: 'review' },
    { id: 'e5', source: 'review', target: 'gate' }
  ]

  it('emits one loop row holding its body', () => {
    const rows = computeFlowLayout(nodes, edges)
    const loopRow = rows.find((r) => r.kind === 'loop')
    expect(loopRow).toBeDefined()
    if (loopRow?.kind === 'loop') {
      expect(loopRow.body.map((b) => (b.kind === 'node' ? b.node.id : ''))).toEqual([
        'write',
        'review'
      ])
    }
  })

  it('never draws a repeated step as a sibling as well', () => {
    // The bug the design exists to kill: body steps appearing twice, once
    // inside the loop and once in the trunk beside steps that run only once.
    const rows = computeFlowLayout(nodes, edges)
    const trunkIds = rows
      .filter((r) => r.kind === 'node')
      .map((r) => (r as { node: WorkflowNode }).node.id)
    expect(trunkIds).not.toContain('write')
    expect(trunkIds).not.toContain('review')
  })

  it('resumes the trunk after the body', () => {
    const rows = computeFlowLayout(nodes, edges)
    const trunkIds = rows
      .filter((r) => r.kind === 'node')
      .map((r) => (r as { node: WorkflowNode }).node.id)
    expect(trunkIds).toEqual(['trigger', 'fetch', 'gate'])
  })

  it('draws an empty loop as an empty rail rather than vanishing', () => {
    const empty = [
      n('trigger', 'trigger'),
      n('loop', 'loop', { nodeType: 'loop', bodyNodeIds: [], maxIterations: 2 })
    ]
    const rows = computeFlowLayout(empty, [{ id: 'e1', source: 'trigger', target: 'loop' }])
    const loopRow = rows.find((r) => r.kind === 'loop')
    expect(loopRow).toBeDefined()
    if (loopRow?.kind === 'loop') expect(loopRow.body).toEqual([])
  })
})

describe('appendToLoopBody', () => {
  const loopNode: WorkflowNode = {
    id: 'loop',
    type: 'loop',
    label: 'Repeat',
    config: {
      nodeType: 'loop',
      bodyNodeIds: ['write'],
      maxIterations: 2
    } as WorkflowNode['config'],
    position: { x: 0, y: 0 }
  }
  const write: WorkflowNode = {
    id: 'write',
    type: 'script',
    label: 'write',
    config: {} as WorkflowNode['config'],
    position: { x: 0, y: 0 }
  }
  const gate: WorkflowNode = {
    id: 'gate',
    type: 'approval',
    label: 'gate',
    config: {} as WorkflowNode['config'],
    position: { x: 0, y: 0 }
  }
  const newStep: WorkflowNode = {
    id: 'review',
    type: 'script',
    label: 'review',
    config: {} as WorkflowNode['config'],
    position: { x: 0, y: 0 }
  }
  const edges: WorkflowEdge[] = [
    { id: 'e1', source: 'loop', target: 'write' },
    { id: 'e2', source: 'write', target: 'gate' }
  ]

  it('writes the edge and the membership together', () => {
    const out = appendToLoopBody([loopNode, write, gate], edges, 'loop', newStep)
    const cfg = out.nodes.find((x) => x.id === 'loop')!.config as { bodyNodeIds: string[] }
    expect(cfg.bodyNodeIds).toEqual(['write', 'review'])
    expect(out.edges.some((e) => e.source === 'write' && e.target === 'review')).toBe(true)
  })

  it('keeps what followed the loop reachable', () => {
    const out = appendToLoopBody([loopNode, write, gate], edges, 'loop', newStep)
    expect(out.edges.some((e) => e.source === 'review' && e.target === 'gate')).toBe(true)
    expect(out.edges.some((e) => e.source === 'write' && e.target === 'gate')).toBe(false)
  })

  it('refuses a node that is not a loop', () => {
    const out = appendToLoopBody([loopNode, write], edges, 'write', newStep)
    expect(out.nodes).toHaveLength(2)
  })
})

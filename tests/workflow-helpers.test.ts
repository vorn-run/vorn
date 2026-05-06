import { describe, it, expect, vi, beforeEach, afterAll } from 'vitest'
import type {
  WorkflowDefinition,
  WorkflowNode,
  WorkflowEdge,
  TriggerConfig
} from '../src/shared/types'

// Stub only crypto.randomUUID for deterministic node IDs, preserving the rest
const originalCrypto = globalThis.crypto
let uuidCounter = 0
vi.stubGlobal('crypto', {
  ...originalCrypto,
  randomUUID: () => `uuid-${++uuidCounter}`
})

afterAll(() => {
  vi.unstubAllGlobals()
})

import {
  getTriggerConfig,
  getTriggerNode,
  getOrderedActionNodes,
  getActionCount,
  isScheduledWorkflow,
  isContextualWorkflow,
  getWorktreeMode,
  getTriggerLabel,
  createTriggerNode,
  createLaunchAgentNode,
  createScriptNode,
  createConditionNode,
  createApprovalNode,
  autoLayoutNodes,
  appendNode,
  removeNode,
  insertNodeBetween,
  insertConditionBetween,
  addParallelBranch,
  computeFlowLayout
} from '../src/renderer/lib/workflow-helpers'

beforeEach(() => {
  uuidCounter = 0
})

function makeTriggerNode(config: TriggerConfig = { triggerType: 'manual' }): WorkflowNode {
  return { id: 'trigger-1', type: 'trigger', label: 'Trigger', config, position: { x: 0, y: 0 } }
}

function makeActionNode(id: string, type: 'launchAgent' | 'script' = 'launchAgent'): WorkflowNode {
  return {
    id,
    type,
    label: `Node ${id}`,
    slug: `node_${id}`,
    config: { agentType: 'claude', projectName: '', projectPath: '' },
    position: { x: 0, y: 0 }
  }
}

function makeEdge(source: string, target: string, id?: string): WorkflowEdge {
  return { id: id || `${source}->${target}`, source, target }
}

function makeWorkflow(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  overrides: Partial<WorkflowDefinition> = {}
): WorkflowDefinition {
  return {
    id: 'wf-1',
    name: 'Test',
    icon: 'Rocket',
    iconColor: '#000',
    nodes,
    edges,
    enabled: true,
    ...overrides
  }
}

// --- Query Helpers ---

describe('getTriggerConfig', () => {
  it('returns config from trigger node', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'recurring', cron: '0 9 * * *' })], [])
    const config = getTriggerConfig(wf)
    expect(config).toEqual({ triggerType: 'recurring', cron: '0 9 * * *' })
  })

  it('returns null when no trigger node', () => {
    const wf = makeWorkflow([makeActionNode('a')], [])
    expect(getTriggerConfig(wf)).toBeNull()
  })
})

describe('getTriggerNode', () => {
  it('finds trigger node', () => {
    const trigger = makeTriggerNode()
    const wf = makeWorkflow([trigger, makeActionNode('a')], [])
    expect(getTriggerNode(wf)).toBe(trigger)
  })

  it('returns undefined when absent', () => {
    const wf = makeWorkflow([makeActionNode('a')], [])
    expect(getTriggerNode(wf)).toBeUndefined()
  })
})

describe('getOrderedActionNodes', () => {
  it('returns BFS order from trigger, excluding trigger', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const b = makeActionNode('b')
    const wf = makeWorkflow([t, a, b], [makeEdge('trigger-1', 'a'), makeEdge('a', 'b')])
    const ordered = getOrderedActionNodes(wf)
    expect(ordered.map((n) => n.id)).toEqual(['a', 'b'])
  })

  it('returns empty when no trigger', () => {
    const wf = makeWorkflow([makeActionNode('a')], [])
    expect(getOrderedActionNodes(wf)).toEqual([])
  })
})

describe('getActionCount', () => {
  it('counts only launchAgent nodes', () => {
    const wf = makeWorkflow(
      [makeTriggerNode(), makeActionNode('a', 'launchAgent'), makeActionNode('b', 'script')],
      []
    )
    expect(getActionCount(wf)).toBe(1)
  })
})

describe('isScheduledWorkflow', () => {
  it('returns true for recurring', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'recurring', cron: '0 9 * * *' })], [])
    expect(isScheduledWorkflow(wf)).toBe(true)
  })

  it('returns true for once', () => {
    const wf = makeWorkflow(
      [makeTriggerNode({ triggerType: 'once', runAt: '2025-01-01T00:00:00Z' })],
      []
    )
    expect(isScheduledWorkflow(wf)).toBe(true)
  })

  it('returns false for manual', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'manual' })], [])
    expect(isScheduledWorkflow(wf)).toBe(false)
  })
})

describe('isContextualWorkflow', () => {
  it('returns true when manual trigger has contextual: true', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'manual', contextual: true })], [])
    expect(isContextualWorkflow(wf)).toBe(true)
  })
  it('returns false for plain manual trigger', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'manual' })], [])
    expect(isContextualWorkflow(wf)).toBe(false)
  })
  it('returns false for scheduled trigger even with contextual flag', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'recurring', cron: '0 0 * * *' })], [])
    expect(isContextualWorkflow(wf)).toBe(false)
  })
  it('returns false when there is no trigger', () => {
    const wf = makeWorkflow([makeActionNode('a')], [])
    expect(isContextualWorkflow(wf)).toBe(false)
  })
})

describe('getWorktreeMode', () => {
  it("returns 'fromContext' when useWorktree === 'fromContext'", () => {
    expect(
      getWorktreeMode({
        agentType: 'claude',
        projectName: '',
        projectPath: '',
        useWorktree: 'fromContext'
      })
    ).toBe('fromContext')
  })
  it("returns 'new' when useWorktree === true and worktreeMode is unset", () => {
    expect(
      getWorktreeMode({
        agentType: 'claude',
        projectName: '',
        projectPath: '',
        useWorktree: true
      })
    ).toBe('new')
  })
  it("returns 'none' when useWorktree is false / undefined", () => {
    expect(getWorktreeMode({ agentType: 'claude', projectName: '', projectPath: '' })).toBe('none')
  })
  it('honors an explicit worktreeMode over the boolean shortcut', () => {
    expect(
      getWorktreeMode({
        agentType: 'claude',
        projectName: '',
        projectPath: '',
        useWorktree: true,
        worktreeMode: 'fromStep',
        worktreeFromStepSlug: 'prep'
      })
    ).toBe('fromStep')
  })
})

describe('getTriggerLabel', () => {
  it('returns "once" for once trigger', () => {
    const wf = makeWorkflow(
      [makeTriggerNode({ triggerType: 'once', runAt: '2025-01-01T00:00:00Z' })],
      []
    )
    expect(getTriggerLabel(wf)).toBe('once')
  })

  it('returns "recurring" for recurring trigger', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'recurring', cron: '0 9 * * *' })], [])
    expect(getTriggerLabel(wf)).toBe('recurring')
  })

  it('returns "on task created" for taskCreated', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'taskCreated' })], [])
    expect(getTriggerLabel(wf)).toBe('on task created')
  })

  it('returns "on status change" for taskStatusChanged', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'taskStatusChanged' })], [])
    expect(getTriggerLabel(wf)).toBe('on status change')
  })

  it('returns undefined for manual', () => {
    const wf = makeWorkflow([makeTriggerNode({ triggerType: 'manual' })], [])
    expect(getTriggerLabel(wf)).toBeUndefined()
  })
})

// --- Node Factories ---

describe('createTriggerNode', () => {
  it('creates manual trigger by default', () => {
    const node = createTriggerNode()
    expect(node.type).toBe('trigger')
    expect(node.label).toBe('Manual Trigger')
    expect(node.config).toEqual({ triggerType: 'manual' })
  })

  it('creates recurring trigger with correct label', () => {
    const node = createTriggerNode({ triggerType: 'recurring', cron: '0 9 * * *' })
    expect(node.label).toBe('Schedule (Recurring)')
  })
})

describe('createLaunchAgentNode', () => {
  it('creates node with defaults', () => {
    const node = createLaunchAgentNode()
    expect(node.type).toBe('launchAgent')
    expect(node.slug).toBe('launch_agent')
    expect((node.config as { agentType: string }).agentType).toBe('claude')
  })

  it('merges overrides', () => {
    const node = createLaunchAgentNode({ agentType: 'copilot' })
    expect((node.config as { agentType: string }).agentType).toBe('copilot')
  })
})

describe('createScriptNode', () => {
  it('creates node with defaults', () => {
    const node = createScriptNode()
    expect(node.type).toBe('script')
    expect((node.config as { scriptType: string }).scriptType).toBe('bash')
  })
})

describe('createConditionNode', () => {
  it('creates node with defaults', () => {
    const node = createConditionNode()
    expect(node.type).toBe('condition')
    expect((node.config as { operator: string }).operator).toBe('equals')
  })
})

describe('createApprovalNode', () => {
  it('creates node with defaults', () => {
    const node = createApprovalNode()
    expect(node.type).toBe('approval')
    expect(node.slug).toBe('approval_gate')
    expect((node.config as { message?: string }).message).toBe('')
  })

  it('merges overrides', () => {
    const node = createApprovalNode({ message: 'please', timeoutMs: 30000 })
    const cfg = node.config as { message: string; timeoutMs: number }
    expect(cfg.message).toBe('please')
    expect(cfg.timeoutMs).toBe(30000)
  })
})

// --- DAG Manipulation ---

describe('autoLayoutNodes', () => {
  it('positions nodes with 140px gap', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const edges = [makeEdge('trigger-1', 'a')]
    const result = autoLayoutNodes([t, a], edges)
    expect(result[0].position.y).toBe(0)
    expect(result[1].position.y).toBe(140)
  })

  it('returns empty array for empty input', () => {
    expect(autoLayoutNodes([], [])).toEqual([])
  })

  it('appends orphan nodes at the end', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const orphan = makeActionNode('orphan')
    const result = autoLayoutNodes([t, a, orphan], [makeEdge('trigger-1', 'a')])
    expect(result.map((n) => n.id)).toEqual(['trigger-1', 'a', 'orphan'])
  })
})

describe('appendNode', () => {
  it('adds node and edge to last terminal node', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const newNode = makeActionNode('b')
    const { nodes, edges } = appendNode([t, a], [makeEdge('trigger-1', 'a')], newNode)
    expect(nodes).toHaveLength(3)
    expect(edges.some((e) => e.source === 'a' && e.target === 'b')).toBe(true)
  })
})

describe('removeNode', () => {
  it('removes node and reconnects predecessors to successors', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const b = makeActionNode('b')
    const edges = [makeEdge('trigger-1', 'a'), makeEdge('a', 'b')]
    const result = removeNode([t, a, b], edges, 'a')
    expect(result.nodes.map((n) => n.id)).toEqual(['trigger-1', 'b'])
    expect(result.edges.some((e) => e.source === 'trigger-1' && e.target === 'b')).toBe(true)
  })

  it('cascade-deletes both branches of a condition up to the join point', () => {
    const t = makeTriggerNode()
    const tail = makeActionNode('tail')
    // Insert a condition between trigger and tail. This sets up two
    // placeholder branches that rejoin at `tail`.
    const inserted = insertConditionBetween(
      [t, tail],
      [makeEdge('trigger-1', 'tail')],
      'trigger-1',
      'tail'
    )
    const condNode = inserted.nodes.find((n) => n.type === 'condition')!

    const result = removeNode(inserted.nodes, inserted.edges, condNode.id)

    // Condition + both branch placeholders gone, only trigger and tail remain.
    expect(result.nodes.map((n) => n.id).sort()).toEqual(['tail', 'trigger-1'])
    // Predecessor reconnected directly to the join.
    expect(result.edges.some((e) => e.source === 'trigger-1' && e.target === 'tail')).toBe(true)
  })

  it('removes a terminal condition without reconnecting (no join)', () => {
    const t = makeTriggerNode()
    const inserted = insertConditionBetween([t], [], 'trigger-1', null)
    const condNode = inserted.nodes.find((n) => n.type === 'condition')!
    const result = removeNode(inserted.nodes, inserted.edges, condNode.id)
    expect(result.nodes.map((n) => n.id)).toEqual(['trigger-1'])
    expect(result.edges).toEqual([])
  })
})

describe('insertNodeBetween', () => {
  it('splits edge and inserts node', () => {
    const t = makeTriggerNode()
    const b = makeActionNode('b')
    const edgeId = 'e1'
    const edges = [{ id: edgeId, source: 'trigger-1', target: 'b' }]
    const newNode = makeActionNode('mid')
    const result = insertNodeBetween([t, b], edges, edgeId, newNode)
    expect(result.nodes).toHaveLength(3)
    expect(result.edges.some((e) => e.source === 'trigger-1' && e.target === 'mid')).toBe(true)
    expect(result.edges.some((e) => e.source === 'mid' && e.target === 'b')).toBe(true)
    // Original edge should be removed
    expect(result.edges.find((e) => e.id === edgeId)).toBeUndefined()
  })

  it('returns unchanged if edge not found', () => {
    const t = makeTriggerNode()
    const result = insertNodeBetween([t], [], 'nonexistent', makeActionNode('x'))
    expect(result.nodes).toEqual([t])
  })
})

describe('insertConditionBetween', () => {
  it('appends condition at end when beforeNodeId is null', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const result = insertConditionBetween([t, a], [makeEdge('trigger-1', 'a')], 'a', null)
    expect(result.nodes.some((n) => n.type === 'condition')).toBe(true)
    // Edge from 'a' to the condition
    const condNode = result.nodes.find((n) => n.type === 'condition')!
    expect(result.edges.some((e) => e.source === 'a' && e.target === condNode.id)).toBe(true)
  })

  it('creates true/false branch placeholders when beforeNodeId given', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const b = makeActionNode('b')
    const result = insertConditionBetween(
      [t, a, b],
      [makeEdge('trigger-1', 'a'), makeEdge('a', 'b')],
      'a',
      'b'
    )
    // Should have condition + 2 branch placeholders added
    const condNodes = result.nodes.filter((n) => n.type === 'condition')
    expect(condNodes).toHaveLength(1)
    // True and false branch edges
    const trueBranch = result.edges.find(
      (e) => e.source === condNodes[0].id && e.conditionBranch === 'true'
    )
    const falseBranch = result.edges.find(
      (e) => e.source === condNodes[0].id && e.conditionBranch === 'false'
    )
    expect(trueBranch).toBeDefined()
    expect(falseBranch).toBeDefined()
  })
})

describe('addParallelBranch', () => {
  it('adds new branch from fork node', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const newNode = makeActionNode('parallel')
    const result = addParallelBranch([t, a], [makeEdge('trigger-1', 'a')], 'trigger-1', newNode)
    expect(result.nodes).toHaveLength(3)
    expect(result.edges.some((e) => e.source === 'trigger-1' && e.target === 'parallel')).toBe(true)
  })
})

// --- Flow Layout ---

describe('computeFlowLayout', () => {
  it('returns empty for empty nodes', () => {
    expect(computeFlowLayout([], [])).toEqual([])
  })

  it('returns node rows for linear chain', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const rows = computeFlowLayout([t, a], [makeEdge('trigger-1', 'a')])
    expect(rows).toHaveLength(2)
    expect(rows[0]).toEqual({ kind: 'node', node: t })
    expect(rows[1]).toEqual({ kind: 'node', node: a })
  })

  it('returns flat node rows when no trigger', () => {
    const a = makeActionNode('a')
    const b = makeActionNode('b')
    const rows = computeFlowLayout([a, b], [])
    expect(rows).toHaveLength(2)
    expect(rows.every((r) => r.kind === 'node')).toBe(true)
  })

  it('produces fork row for branching', () => {
    const t = makeTriggerNode()
    const a = makeActionNode('a')
    const b = makeActionNode('b')
    const c = makeActionNode('c')
    const edges = [
      makeEdge('trigger-1', 'a'),
      makeEdge('trigger-1', 'b'),
      makeEdge('a', 'c'),
      makeEdge('b', 'c')
    ]
    const rows = computeFlowLayout([t, a, b, c], edges)
    // Trigger is first row, then fork, then join
    expect(rows[0]).toEqual({ kind: 'node', node: t })
    const forkRow = rows.find((r) => r.kind === 'fork')
    expect(forkRow).toBeDefined()
  })

  it('appends orphan nodes not reachable from trigger', () => {
    const t = makeTriggerNode()
    const orphan = makeActionNode('orphan')
    const rows = computeFlowLayout([t, orphan], [])
    expect(rows).toHaveLength(2)
  })
})

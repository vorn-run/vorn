import {
  WorkflowDefinition,
  WorkflowNode,
  WorkflowEdge,
  TriggerConfig,
  LaunchAgentConfig,
  ScriptConfig,
  ConditionConfig,
  ApprovalConfig,
  WorkflowNodePosition
} from '../../shared/types'
import { slugify } from './template-vars'

// --- Flow Layout Types ---

export type FlowRow =
  | { kind: 'node'; node: WorkflowNode }
  | { kind: 'fork'; forkNodeId: string; branches: FlowRow[][]; joinNodeId?: string }

// --- Graph Adjacency Helpers ---

function buildSuccessorsMap(edges: WorkflowEdge[]): Map<string, string[]> {
  const map = new Map<string, string[]>()
  for (const edge of edges) {
    const succs = map.get(edge.source) || []
    succs.push(edge.target)
    map.set(edge.source, succs)
  }
  return map
}

function findJoinPoint(
  _forkNodeId: string,
  children: string[],
  successorsMap: Map<string, string[]>
): string | null {
  if (children.length <= 1) return null

  const reachableSets = children.map((childId) => {
    const reachable = new Set<string>()
    const queue = [childId]
    while (queue.length > 0) {
      const current = queue.shift()!
      if (reachable.has(current)) continue
      reachable.add(current)
      for (const next of successorsMap.get(current) || []) {
        queue.push(next)
      }
    }
    return reachable
  })

  const childrenSet = new Set(children)
  const visited = new Set<string>()
  const queue = [...children]

  while (queue.length > 0) {
    const current = queue.shift()!
    if (visited.has(current)) continue
    visited.add(current)

    if (!childrenSet.has(current) && reachableSets.every((set) => set.has(current))) {
      return current
    }

    for (const next of successorsMap.get(current) || []) {
      queue.push(next)
    }
  }

  return null
}

// --- Existing helpers (unchanged) ---

export function getTriggerConfig(wf: WorkflowDefinition): TriggerConfig | null {
  const triggerNode = wf.nodes.find((n) => n.type === 'trigger')
  if (!triggerNode) return null
  return triggerNode.config as TriggerConfig
}

export function getTriggerNode(wf: WorkflowDefinition): WorkflowNode | undefined {
  return wf.nodes.find((n) => n.type === 'trigger')
}

export function getOrderedActionNodes(wf: WorkflowDefinition): WorkflowNode[] {
  const triggerNode = getTriggerNode(wf)
  if (!triggerNode) return []

  const nodeMap = new Map(wf.nodes.map((n) => [n.id, n]))
  const childrenMap = new Map<string, string[]>()
  for (const edge of wf.edges) {
    const children = childrenMap.get(edge.source) || []
    children.push(edge.target)
    childrenMap.set(edge.source, children)
  }

  const ordered: WorkflowNode[] = []
  const visited = new Set<string>()
  const queue = [triggerNode.id]
  visited.add(triggerNode.id)

  while (queue.length > 0) {
    const current = queue.shift()!
    const node = nodeMap.get(current)
    if (node && node.type !== 'trigger') {
      ordered.push(node)
    }
    const children = childrenMap.get(current) || []
    for (const childId of children) {
      if (!visited.has(childId)) {
        visited.add(childId)
        queue.push(childId)
      }
    }
  }

  return ordered
}

export function getActionCount(wf: WorkflowDefinition): number {
  return wf.nodes.filter((n) => n.type === 'launchAgent').length
}

export function isScheduledWorkflow(wf: WorkflowDefinition): boolean {
  const trigger = getTriggerConfig(wf)
  return trigger != null && trigger.triggerType !== 'manual'
}

export function isContextualWorkflow(wf: WorkflowDefinition): boolean {
  const trigger = getTriggerConfig(wf)
  return trigger?.triggerType === 'manual' && trigger.contextual === true
}

export type WorktreeMode = 'none' | 'new' | 'fromStep' | 'existing' | 'fromContext'

export function getWorktreeMode(cfg: LaunchAgentConfig): WorktreeMode {
  if (cfg.useWorktree === 'fromContext') return 'fromContext'
  return cfg.worktreeMode ?? (cfg.useWorktree === true ? 'new' : 'none')
}

export function getTriggerLabel(wf: WorkflowDefinition): string | undefined {
  const trigger = getTriggerConfig(wf)
  if (!trigger) return undefined
  if (trigger.triggerType === 'once') return 'once'
  if (trigger.triggerType === 'recurring') return 'recurring'
  if (trigger.triggerType === 'taskCreated') return 'on task created'
  if (trigger.triggerType === 'taskStatusChanged') return 'on status change'
  return undefined
}

export function createTriggerNode(config: TriggerConfig = { triggerType: 'manual' }): WorkflowNode {
  const labelMap: Record<string, string> = {
    manual: 'Manual Trigger',
    once: 'Schedule (Once)',
    recurring: 'Schedule (Recurring)',
    taskCreated: 'When Task Created',
    taskStatusChanged: 'When Task Status Changes'
  }
  return {
    id: crypto.randomUUID(),
    type: 'trigger',
    label: labelMap[config.triggerType] || 'Trigger',
    config,
    position: { x: 0, y: 0 }
  }
}

export function createLaunchAgentNode(config: Partial<LaunchAgentConfig> = {}): WorkflowNode {
  return {
    id: crypto.randomUUID(),
    type: 'launchAgent',
    label: 'Launch Agent',
    slug: slugify('Launch Agent'),
    config: {
      agentType: 'claude',
      projectName: '',
      projectPath: '',
      headless: true,
      ...config
    } as LaunchAgentConfig,
    position: { x: 0, y: 0 }
  }
}

export function createScriptNode(config: Partial<ScriptConfig> = {}): WorkflowNode {
  return {
    id: crypto.randomUUID(),
    type: 'script',
    label: 'Execute Script',
    slug: slugify('Execute Script'),
    config: {
      scriptType: 'bash',
      scriptContent: '# Write your script here\n',
      projectName: '',
      projectPath: '',
      ...config
    } as ScriptConfig,
    position: { x: 0, y: 0 }
  }
}

export function createApprovalNode(config: Partial<ApprovalConfig> = {}): WorkflowNode {
  return {
    id: crypto.randomUUID(),
    type: 'approval',
    label: 'Approval Gate',
    slug: slugify('Approval Gate'),
    config: {
      message: '',
      ...config
    } as ApprovalConfig,
    position: { x: 0, y: 0 }
  }
}

export function createCallConnectorActionNode(
  config: Partial<import('../../shared/types').CallConnectorActionConfig> = {}
): WorkflowNode {
  return {
    id: crypto.randomUUID(),
    type: 'callConnectorAction',
    label: 'Connector Action',
    slug: slugify('Connector Action'),
    config: {
      nodeType: 'callConnectorAction',
      connectionId: '',
      action: '',
      args: {},
      ...config
    } as import('../../shared/types').CallConnectorActionConfig,
    position: { x: 0, y: 0 }
  }
}

export function createConditionNode(config: Partial<ConditionConfig> = {}): WorkflowNode {
  return {
    id: crypto.randomUUID(),
    type: 'condition',
    label: 'Condition',
    slug: slugify('Condition'),
    config: {
      variable: '',
      operator: 'equals',
      value: '',
      ...config
    } as ConditionConfig,
    position: { x: 0, y: 0 }
  }
}

/**
 * Insert a condition node between two nodes (or at the end).
 * Creates the condition + placeholder branch-start nodes for true/false.
 * If there's a downstream node, both branches rejoin at it via their placeholders.
 * This avoids pointing both branches directly at the same target, which would
 * cause markSkippedBranch to incorrectly skip the shared join node.
 */
export function insertConditionBetween(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  afterNodeId: string,
  beforeNodeId: string | null
): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  const conditionNode = createConditionNode()

  const newNodes = [...nodes, conditionNode]
  let newEdges = [...edges]

  if (beforeNodeId) {
    // Create placeholder nodes for each branch so they don't share a target
    const trueBranch = createScriptNode({ scriptContent: '# True branch\nexit 0\n' })
    trueBranch.label = 'True Branch'
    trueBranch.slug = slugify('True Branch')
    const falseBranch = createScriptNode({ scriptContent: '# False branch\nexit 0\n' })
    falseBranch.label = 'False Branch'
    falseBranch.slug = slugify('False Branch')
    newNodes.push(trueBranch, falseBranch)

    // Remove the direct edge between after → before
    newEdges = newEdges.filter((e) => !(e.source === afterNodeId && e.target === beforeNodeId))
    // after → condition
    newEdges.push({ id: crypto.randomUUID(), source: afterNodeId, target: conditionNode.id })
    // condition → true placeholder
    newEdges.push({
      id: crypto.randomUUID(),
      source: conditionNode.id,
      target: trueBranch.id,
      conditionBranch: 'true'
    })
    // condition → false placeholder
    newEdges.push({
      id: crypto.randomUUID(),
      source: conditionNode.id,
      target: falseBranch.id,
      conditionBranch: 'false'
    })
    // Both placeholders rejoin at the downstream node
    newEdges.push({ id: crypto.randomUUID(), source: trueBranch.id, target: beforeNodeId })
    newEdges.push({ id: crypto.randomUUID(), source: falseBranch.id, target: beforeNodeId })
  } else {
    // Appending at the end — just connect after → condition
    newEdges.push({ id: crypto.randomUUID(), source: afterNodeId, target: conditionNode.id })
  }

  return { nodes: autoLayoutNodes(newNodes, newEdges), edges: newEdges }
}

export function autoLayoutNodes(nodes: WorkflowNode[], edges: WorkflowEdge[]): WorkflowNode[] {
  if (nodes.length === 0) return nodes

  const triggerNode = nodes.find((n) => n.type === 'trigger')
  const ordered = triggerNode ? [triggerNode] : []

  const childrenMap = new Map<string, string[]>()
  for (const edge of edges) {
    const children = childrenMap.get(edge.source) || []
    children.push(edge.target)
    childrenMap.set(edge.source, children)
  }

  const nodeMap = new Map(nodes.map((n) => [n.id, n]))
  const visited = new Set(ordered.map((n) => n.id))
  const queue = ordered.map((n) => n.id)

  while (queue.length > 0) {
    const current = queue.shift()!
    const children = childrenMap.get(current) || []
    for (const childId of children) {
      if (!visited.has(childId)) {
        visited.add(childId)
        queue.push(childId)
        const node = nodeMap.get(childId)
        if (node) ordered.push(node)
      }
    }
  }

  for (const node of nodes) {
    if (!visited.has(node.id)) {
      ordered.push(node)
    }
  }

  const NODE_GAP = 80

  return ordered.map((node, index) => ({
    ...node,
    position: { x: 0, y: index * (60 + NODE_GAP) } as WorkflowNodePosition
  }))
}

export function insertNodeBetween(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  edgeId: string,
  newNode: WorkflowNode
): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  const edge = edges.find((e) => e.id === edgeId)
  if (!edge) return { nodes, edges }

  const newEdges = edges.filter((e) => e.id !== edgeId)
  newEdges.push(
    { id: crypto.randomUUID(), source: edge.source, target: newNode.id },
    { id: crypto.randomUUID(), source: newNode.id, target: edge.target }
  )

  const newNodes = [...nodes, newNode]
  return { nodes: autoLayoutNodes(newNodes, newEdges), edges: newEdges }
}

export function appendNode(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  newNode: WorkflowNode
): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  const nodesWithOutgoing = new Set(edges.map((e) => e.source))
  const lastNode = [...nodes].reverse().find((n) => !nodesWithOutgoing.has(n.id))

  const newEdges = [...edges]
  if (lastNode) {
    newEdges.push({ id: crypto.randomUUID(), source: lastNode.id, target: newNode.id })
  }

  const newNodes = [...nodes, newNode]
  return { nodes: autoLayoutNodes(newNodes, newEdges), edges: newEdges }
}

export function removeNode(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  nodeId: string
): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  const nodeToRemove = nodes.find((n) => n.id === nodeId)

  // Condition nodes own two branch subtrees. Removing just the condition
  // and reconnecting predecessors to both branches would turn the if/else
  // into two parallel paths that silently both run — almost never what the
  // user wants when they hit "remove". Cascade the delete down both
  // branches up to the join point, and reconnect predecessors to the join
  // (or leave them as leaves if the condition was at the end of a chain).
  if (nodeToRemove?.type === 'condition') {
    const successorsMap = buildSuccessorsMap(edges)
    const branchStarts = successorsMap.get(nodeId) ?? []
    const joinId = findJoinPoint(nodeId, branchStarts, successorsMap)
    const toRemove = collectBranchSubtree(branchStarts, joinId, successorsMap)
    toRemove.add(nodeId)

    const predecessors = edges.filter((e) => e.target === nodeId).map((e) => e.source)
    const remainingEdges = edges.filter((e) => !toRemove.has(e.source) && !toRemove.has(e.target))
    if (joinId) {
      for (const pred of predecessors) {
        remainingEdges.push({ id: crypto.randomUUID(), source: pred, target: joinId })
      }
    }
    const remainingNodes = nodes.filter((n) => !toRemove.has(n.id))
    return { nodes: autoLayoutNodes(remainingNodes, remainingEdges), edges: remainingEdges }
  }

  const incomingEdges = edges.filter((e) => e.target === nodeId)
  const outgoingEdges = edges.filter((e) => e.source === nodeId)

  const newEdges = [...edges.filter((e) => e.source !== nodeId && e.target !== nodeId)]

  for (const incoming of incomingEdges) {
    for (const outgoing of outgoingEdges) {
      newEdges.push({ id: crypto.randomUUID(), source: incoming.source, target: outgoing.target })
    }
  }

  const newNodes = nodes.filter((n) => n.id !== nodeId)
  return { nodes: autoLayoutNodes(newNodes, newEdges), edges: newEdges }
}

/**
 * Walk from each branch start forward, stopping at (but not including) the
 * shared join point. Returns the set of node ids that belong to the condition's
 * owned subtree and can be safely deleted along with it.
 */
function collectBranchSubtree(
  branchStarts: string[],
  joinId: string | null,
  successorsMap: Map<string, string[]>
): Set<string> {
  const collected = new Set<string>()
  const queue = [...branchStarts]
  while (queue.length > 0) {
    const current = queue.shift()!
    if (current === joinId) continue
    if (collected.has(current)) continue
    collected.add(current)
    for (const next of successorsMap.get(current) ?? []) {
      if (next !== joinId) queue.push(next)
    }
  }
  return collected
}

// --- Flow Layout ---

function collectNodeIds(rows: FlowRow[]): Set<string> {
  const ids = new Set<string>()
  for (const row of rows) {
    if (row.kind === 'node') {
      ids.add(row.node.id)
    } else {
      for (const branch of row.branches) {
        collectNodeIds(branch).forEach((id) => ids.add(id))
      }
    }
  }
  return ids
}

export function computeFlowLayout(nodes: WorkflowNode[], edges: WorkflowEdge[]): FlowRow[] {
  if (nodes.length === 0) return []

  const nodeMap = new Map(nodes.map((n) => [n.id, n]))
  const successorsMap = buildSuccessorsMap(edges)
  const triggerNode = nodes.find((n) => n.type === 'trigger')

  if (!triggerNode) return nodes.map((n) => ({ kind: 'node' as const, node: n }))

  const rows = buildFlowFromNode(triggerNode.id, null, nodeMap, successorsMap)

  // Append orphan nodes not reachable from the trigger
  const visited = collectNodeIds(rows)
  for (const node of nodes) {
    if (!visited.has(node.id)) {
      rows.push({ kind: 'node', node })
    }
  }

  return rows
}

function buildFlowFromNode(
  startId: string,
  stopBeforeId: string | null,
  nodeMap: Map<string, WorkflowNode>,
  successorsMap: Map<string, string[]>
): FlowRow[] {
  const rows: FlowRow[] = []
  let currentId: string | null = startId

  while (currentId) {
    if (currentId === stopBeforeId) break

    const node = nodeMap.get(currentId)
    if (!node) break

    const successors: string[] = successorsMap.get(currentId) || []

    if (successors.length <= 1) {
      rows.push({ kind: 'node', node })
      currentId = successors[0] || null
    } else {
      rows.push({ kind: 'node', node })

      const joinNodeId = findJoinPoint(currentId, successors, successorsMap)
      const branches = successors.map((childId: string) =>
        buildFlowFromNode(childId, joinNodeId, nodeMap, successorsMap)
      )

      rows.push({
        kind: 'fork',
        forkNodeId: currentId,
        branches,
        joinNodeId: joinNodeId || undefined
      })

      currentId = joinNodeId
    }
  }

  return rows
}

// --- Parallel Branch Operations ---

export function appendNodeAfter(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  afterNodeId: string,
  newNode: WorkflowNode
): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  const newNodes = [...nodes, newNode]
  const newEdges = [...edges, { id: crypto.randomUUID(), source: afterNodeId, target: newNode.id }]
  return { nodes: autoLayoutNodes(newNodes, newEdges), edges: newEdges }
}

export function insertBeforeFork(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  forkNodeId: string,
  newNode: WorkflowNode
): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  const newEdges = edges.map((e) =>
    e.source === forkNodeId ? { id: crypto.randomUUID(), source: newNode.id, target: e.target } : e
  )
  newEdges.push({ id: crypto.randomUUID(), source: forkNodeId, target: newNode.id })

  const newNodes = [...nodes, newNode]
  return { nodes: autoLayoutNodes(newNodes, newEdges), edges: newEdges }
}

export function addParallelBranch(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  forkFromId: string,
  newNode: WorkflowNode
): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  const successorsMap = buildSuccessorsMap(edges)
  const successors = successorsMap.get(forkFromId) || []

  const newNodes = [...nodes, newNode]
  const newEdges = [...edges]

  newEdges.push({ id: crypto.randomUUID(), source: forkFromId, target: newNode.id })

  if (successors.length === 0) {
    // Terminal — no convergence
  } else if (successors.length === 1) {
    let joinTarget: string | null = null
    let current = successors[0]
    while (true) {
      const succs = successorsMap.get(current) || []
      if (succs.length === 0) {
        break
      } else if (succs.length === 1) {
        joinTarget = succs[0]
        break
      } else {
        const jp = findJoinPoint(current, succs, successorsMap)
        if (jp) {
          current = jp
        } else {
          break
        }
      }
    }

    if (joinTarget) {
      newEdges.push({ id: crypto.randomUUID(), source: newNode.id, target: joinTarget })
    }
  } else {
    const joinNodeId = findJoinPoint(forkFromId, successors, successorsMap)
    if (joinNodeId) {
      newEdges.push({ id: crypto.randomUUID(), source: newNode.id, target: joinNodeId })
    }
  }

  return { nodes: autoLayoutNodes(newNodes, newEdges), edges: newEdges }
}

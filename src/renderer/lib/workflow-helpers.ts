import {
  WorkflowDefinition,
  WorkflowNode,
  WorkflowEdge,
  TriggerConfig,
  LaunchAgentConfig,
  ScriptConfig,
  ConditionConfig,
  ApprovalConfig,
  LoopConfig,
  WorkflowNodePosition,
  WorkflowInputDef,
  TaskConfig,
  TerminalSession
} from '../../shared/types'
import { slugify } from './template-vars'

// --- Flow Layout Types ---

export type FlowRow =
  | { kind: 'node'; node: WorkflowNode }
  | { kind: 'fork'; forkNodeId: string; branches: FlowRow[][]; joinNodeId?: string }
  /**
   * A loop and the steps it repeats, drawn as one thing.
   *
   * The body is lifted out of the trunk and nested here, the same move a fork
   * already makes with its branches. Without it the repeated steps render as
   * ordinary siblings of steps that run once, and the canvas stops being a
   * picture of what happens.
   */
  | { kind: 'loop'; loopNode: WorkflowNode; body: FlowRow[] }

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

/** Manual-run parameters the workflow declares on its trigger, if any. */
export function getWorkflowInputs(wf: WorkflowDefinition): WorkflowInputDef[] {
  const trigger = getTriggerConfig(wf)
  if (trigger?.triggerType !== 'manual') return []
  // Match the template parser's identifier grammar. Invalid or half-authored
  // keys cannot be referenced as `{{inputs.<key>}}`, so they must not cause a
  // run prompt or enter the values map.
  return (trigger.inputs ?? []).filter((i) => /^[a-zA-Z_]\w*$/.test(i.key))
}

/**
 * What a launching surface already knows about the run. A card or terminal
 * right-click supplies these; a sidebar or palette launch does not.
 */
export interface ManualRunContext {
  task?: TaskConfig
  source?: TerminalSession
}

/**
 * Whether starting this workflow requires asking the user for something first.
 *
 * Contextual workflows need a source folder/branch — unless the launching
 * surface already supplied one. Declared run inputs always need the user,
 * since a run is the only moment those values can be supplied.
 *
 * Takes the launch context so there is exactly one definition of "must
 * prompt": call sites that pass a source and call sites that don't share this
 * predicate rather than each hand-rolling half of it.
 */
export function needsRunPrompt(wf: WorkflowDefinition, ctx?: ManualRunContext): boolean {
  const hasSource = !!(ctx?.task || ctx?.source)
  return (isContextualWorkflow(wf) && !hasSource) || getWorkflowInputs(wf).length > 0
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
    taskStatusChanged: 'When Task Status Changes',
    connectorPoll: 'Connector Poll',
    webhook: 'Webhook'
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

/**
 * Add a step into a loop's body.
 *
 * Writes the edge and the membership together, because they are one fact. The
 * checkbox list this replaces let them drift: a step could sit in bodyNodeIds
 * while the graph ran it somewhere else entirely.
 */
export function appendToLoopBody(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  loopNodeId: string,
  newNode: WorkflowNode
): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  const loop = nodes.find((n) => n.id === loopNodeId)
  if (!loop || loop.type !== 'loop') return { nodes, edges }

  const config = loop.config as LoopConfig
  const body = config.bodyNodeIds ?? []
  const lastId = body[body.length - 1] ?? loopNodeId

  // The new step goes after the current last body step, taking over whatever
  // that step pointed at so the rest of the workflow still follows the loop.
  const onward = edges.filter((e) => e.source === lastId)
  const nextEdges: WorkflowEdge[] = [
    ...edges.filter((e) => e.source !== lastId),
    { id: crypto.randomUUID(), source: lastId, target: newNode.id },
    ...onward.map((e) => ({ ...e, id: crypto.randomUUID(), source: newNode.id }))
  ]

  const nextNodes = nodes.map((n) =>
    n.id === loopNodeId
      ? { ...n, config: { ...config, bodyNodeIds: [...body, newNode.id] } as LoopConfig }
      : n
  )

  // Laid out like every other insert helper: without this the new step keeps
  // its default (0,0) position and the persisted layout drifts from what the
  // rest of the editor produces.
  const withNew = [...nextNodes, newNode]
  return { nodes: placeNewNodes(nodes, withNew, nextEdges), edges: nextEdges }
}

/**
 * The loop that owns an insertion point inside a rail.
 *
 * The + inside a loop reports the step it sits under, which is the loop itself
 * while the body is empty and the last body step afterwards. Resolving that to
 * a loop is the kind of branch that quietly rots inside a click handler, so it
 * lives here where it can be tested.
 */
export function loopOwningInsertPoint(
  nodes: WorkflowNode[],
  afterNodeId: string
): WorkflowNode | undefined {
  const direct = nodes.find((n) => n.id === afterNodeId && n.type === 'loop')
  if (direct) return direct

  return nodes.find(
    (n) => n.type === 'loop' && ((n.config as LoopConfig).bodyNodeIds ?? []).includes(afterNodeId)
  )
}

export function createLoopNode(config: Partial<LoopConfig> = {}): WorkflowNode {
  return {
    id: crypto.randomUUID(),
    type: 'loop',
    label: 'Repeat Steps',
    slug: slugify('Repeat Steps'),
    config: {
      nodeType: 'loop',
      bodyNodeIds: [],
      maxIterations: 2,
      ...config
    } as LoopConfig,
    position: { x: 0, y: 0 }
  }
}

/**
 * Steps a loop is allowed to repeat: everything downstream of it.
 *
 * A loop drives its own body, so offering an upstream step would let someone
 * build a workflow that re-runs work the loop's own inputs depend on.
 */
export function nodesAfter(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  nodeId: string
): WorkflowNode[] {
  const successors = buildSuccessorsMap(edges)
  const nodeMap = new Map(nodes.map((n) => [n.id, n]))
  const found: WorkflowNode[] = []
  const seen = new Set<string>([nodeId])
  const queue = [...(successors.get(nodeId) ?? [])]

  while (queue.length > 0) {
    const id = queue.shift()!
    if (seen.has(id)) continue
    seen.add(id)
    const node = nodeMap.get(id)
    if (node && node.type !== 'trigger') found.push(node)
    queue.push(...(successors.get(id) ?? []))
  }

  return found
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

/** The {{trigger.*}} namespace of a webhook run, rebuilt from the event's stored payload. */
export function webhookTriggerFromItem(
  connectorItem: import('../../shared/types').ConnectorItemContext | undefined
): import('../../shared/types').WorkflowExecutionContext['trigger'] | undefined {
  if (connectorItem?.connectorId !== 'webhook') return undefined
  const raw = connectorItem.raw as {
    body?: unknown
    headers?: Record<string, string>
    query?: Record<string, string>
    method?: string
  }
  return {
    type: 'webhook' as const,
    body: raw.body,
    headers: raw.headers,
    query: raw.query,
    method: raw.method
  }
}

/**
 * The run context for a scheduler-delivered event. A webhook event rides the
 * connector pipe for durability, so its payload is lifted into the trigger
 * namespace here while the connectorItem keeps the lease machinery working.
 */
export function schedulerExecutionContext(
  connectorItem: import('../../shared/types').ConnectorItemContext | undefined,
  inputs: Record<string, unknown> | undefined
): import('../../shared/types').WorkflowExecutionContext | undefined {
  if (!connectorItem && !inputs) return undefined
  const trigger = webhookTriggerFromItem(connectorItem)
  return { connectorItem, inputs, ...(trigger && { trigger }) }
}

/** Steps that can swap type in place; condition, loop, and trigger own structure or their own path. */
export const REPLACEABLE_NODE_TYPES: ReadonlySet<string> = new Set([
  'launchAgent',
  'script',
  'approval',
  'createTaskFromItem',
  'callConnectorAction',
  'httpRequest'
])

/** The default config for each trigger type, used by the form and the library. */
export function switchTriggerType(type: TriggerConfig['triggerType']): TriggerConfig {
  switch (type) {
    case 'manual':
      return { triggerType: 'manual' }
    case 'once':
      return { triggerType: 'once', runAt: new Date().toISOString() }
    case 'recurring':
      return { triggerType: 'recurring', cron: '0 9 * * *' }
    case 'taskCreated':
      return { triggerType: 'taskCreated' }
    case 'taskStatusChanged':
      return { triggerType: 'taskStatusChanged' }
    case 'connectorPoll':
      return { triggerType: 'connectorPoll', connectionId: '', event: '', cron: '*/5 * * * *' }
    case 'webhook':
      return { triggerType: 'webhook', method: 'POST', token: crypto.randomUUID() }
  }
}

export function createHttpRequestNode(
  config: Partial<import('../../shared/types').HttpRequestConfig> = {}
): WorkflowNode {
  return {
    id: crypto.randomUUID(),
    type: 'httpRequest',
    label: 'HTTP Request',
    slug: slugify('HTTP Request'),
    config: {
      nodeType: 'httpRequest',
      method: 'GET',
      url: '',
      headers: {},
      body: '',
      ...config
    } as import('../../shared/types').HttpRequestConfig,
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

  return { nodes: placeNewNodes(nodes, newNodes, newEdges), edges: newEdges }
}

/** Every node id reachable by following edges forward from `startId`. */
function reachableFrom(startId: string, edges: WorkflowEdge[]): Set<string> {
  const successors = buildSuccessorsMap(edges)
  const seen = new Set<string>()
  const queue = [...(successors.get(startId) ?? [])]
  while (queue.length > 0) {
    const id = queue.shift()!
    if (seen.has(id)) continue
    seen.add(id)
    queue.push(...(successors.get(id) ?? []))
  }
  return seen
}

/** Whether every stored position is still the untouched seed column (x = 0 everywhere). */
export function positionsAreSeed(nodes: WorkflowNode[]): boolean {
  return nodes.every((n) => !n.position || n.position.x === 0)
}

/** Seed definitions reflow into the legacy column; arranged ones keep their positions and only new nodes are placed. */
export function placeNewNodes(
  prevNodes: WorkflowNode[],
  nextNodes: WorkflowNode[],
  edges: WorkflowEdge[]
): WorkflowNode[] {
  if (positionsAreSeed(prevNodes)) return autoLayoutNodes(nextNodes, edges)

  const prevIds = new Set(prevNodes.map((n) => n.id))
  // Positions accumulate so chained new nodes hang off placed predecessors, siblings fanned apart.
  const placed = new Map<string, WorkflowNodePosition>()
  for (const node of nextNodes) {
    if (prevIds.has(node.id)) placed.set(node.id, node.position)
  }
  const siblingCount = new Map<string, number>()
  const remaining = nextNodes.filter((n) => !prevIds.has(n.id))
  while (remaining.length > 0) {
    const readyIndex = remaining.findIndex((n) => {
      const incoming = edges.find((e) => e.target === n.id)
      return !incoming || placed.has(incoming.source)
    })
    const index = readyIndex === -1 ? 0 : readyIndex
    const node = remaining.splice(index, 1)[0]
    const incoming = edges.find((e) => e.target === node.id)
    const base = (incoming && placed.get(incoming.source)) ?? { x: 0, y: 0 }
    const sibling = incoming ? (siblingCount.get(incoming.source) ?? 0) : 0
    if (incoming) siblingCount.set(incoming.source, sibling + 1)
    let candidate = { x: base.x + sibling * 320, y: base.y + 140 }
    // Probe downward until the spot is free, ignoring this node's own
    // downstream — those get shifted out of the way below.
    const downstream = reachableFrom(node.id, edges)
    const occupied = (p: WorkflowNodePosition): boolean =>
      [...placed.entries()].some(
        ([id, o]) => !downstream.has(id) && Math.abs(o.x - p.x) < 300 && Math.abs(o.y - p.y) < 120
      )
    for (let tries = 0; occupied(candidate) && tries < 50; tries++) {
      candidate = { x: candidate.x, y: candidate.y + 140 }
    }
    placed.set(node.id, candidate)
  }

  // Inserting mid-chain must make room: downstream nodes sitting where a new
  // node landed move down by one pitch instead of being covered.
  const newIds = new Set(nextNodes.filter((n) => !prevIds.has(n.id)).map((n) => n.id))
  const shifted = new Set<string>()
  for (const newId of newIds) {
    const newPos = placed.get(newId)!
    for (const id of reachableFrom(newId, edges)) {
      if (newIds.has(id)) continue
      const pos = placed.get(id)
      if (pos && pos.y <= newPos.y + 100) shifted.add(id)
    }
  }

  return nextNodes.map((node) => {
    if (newIds.has(node.id)) return { ...node, position: placed.get(node.id)! }
    if (shifted.has(node.id))
      return { ...node, position: { x: node.position.x, y: node.position.y + 140 } }
    return node
  })
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
    // The branch tag rides the first half of the split so the fork keeps telling branches apart.
    {
      id: crypto.randomUUID(),
      source: edge.source,
      target: newNode.id,
      ...(edge.conditionBranch && { conditionBranch: edge.conditionBranch })
    },
    { id: crypto.randomUUID(), source: newNode.id, target: edge.target }
  )

  const newNodes = [...nodes, newNode]
  return { nodes: placeNewNodes(nodes, newNodes, newEdges), edges: newEdges }
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
  return { nodes: placeNewNodes(nodes, newNodes, newEdges), edges: newEdges }
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
    return { nodes: placeNewNodes(nodes, remainingNodes, remainingEdges), edges: remainingEdges }
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
  return { nodes: placeNewNodes(nodes, newNodes, newEdges), edges: newEdges }
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
    } else if (row.kind === 'loop') {
      ids.add(row.loopNode.id)
      collectNodeIds(row.body).forEach((id) => ids.add(id))
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
  successorsMap: Map<string, string[]>,
  // Nothing validates the graph for cycles, and this walk is the canvas: an
  // edge pointing back at an ancestor used to spin here forever and hang the
  // editor with no way back to the workflow that caused it. Stopping at a node
  // we have already drawn renders such a graph as a truncated chain, which is
  // wrong but visible and recoverable.
  seen: Set<string> = new Set()
): FlowRow[] {
  const rows: FlowRow[] = []
  let currentId: string | null = startId

  while (currentId) {
    if (currentId === stopBeforeId) break
    if (seen.has(currentId)) break
    seen.add(currentId)

    const node = nodeMap.get(currentId)
    if (!node) break

    const successors: string[] = successorsMap.get(currentId) || []

    if (node.type === 'loop') {
      const bodyIds = ((node.config as LoopConfig).bodyNodeIds ?? []).filter((id) =>
        nodeMap.has(id)
      )
      const body: FlowRow[] = bodyIds.map((id) => ({
        kind: 'node' as const,
        node: nodeMap.get(id)!
      }))
      // The body is drawn inside the loop, so mark it seen: walking into it
      // again from the trunk would draw every repeated step twice.
      for (const id of bodyIds) seen.add(id)

      rows.push({ kind: 'loop', loopNode: node, body })

      // Resume after the body, which is where the workflow actually continues.
      let next: string | null = successors[0] || null
      while (next && bodyIds.includes(next)) {
        const onward: string[] = successorsMap.get(next) || []
        next = onward[0] || null
      }
      currentId = next
      continue
    }

    if (successors.length <= 1) {
      rows.push({ kind: 'node', node })
      currentId = successors[0] || null
    } else {
      rows.push({ kind: 'node', node })

      const joinNodeId = findJoinPoint(currentId, successors, successorsMap)
      // Each branch walks with its own `seen`, seeded from the trunk: two
      // branches legitimately reconverge on the join, and sharing one set
      // would let whichever branch drew first suppress the other.
      const branches = successors.map((childId: string) =>
        buildFlowFromNode(childId, joinNodeId, nodeMap, successorsMap, new Set(seen))
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
  return { nodes: placeNewNodes(nodes, newNodes, newEdges), edges: newEdges }
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
  return { nodes: placeNewNodes(nodes, newNodes, newEdges), edges: newEdges }
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

  return { nodes: placeNewNodes(nodes, newNodes, newEdges), edges: newEdges }
}

import { Position, type Edge, type Node } from '@xyflow/react'
import { LoopConfig, WorkflowEdge, WorkflowNode } from '../../shared/types'
import { stepPreview } from '../components/workflow-editor/node-visuals'
import { computeFlowLayout, FlowRow } from './workflow-helpers'

// Projects a workflow definition into canvas elements; the definition stays the source of truth.

/** The anchor id that opens the library in trigger scope. */
export const TRIGGER_ANCHOR_ID = '__TRIGGER__'

export const TRIGGER_ANCHOR = {
  afterNodeId: TRIGGER_ANCHOR_ID,
  beforeNodeId: null,
  insideBranch: false,
  bodyOnly: false
}

export const CARD_WIDTH = 280
export const LOOP_WIDTH = 312
/** Horizontal gap between fork branches. */
const BRANCH_GAP = 56
/** Vertical gap between consecutive steps (room for the edge). */
const ROW_GAP = 56

/** Only the id: content, selection, and status come from context, keeping the node array stable. */
export interface CanvasNodeData extends Record<string, unknown> {
  nodeId: string
}

/** The + that trails every leaf, and what an insertion there means. */
export interface AddStepNodeData extends Record<string, unknown> {
  afterNodeId: string
  insideBranch: boolean
}

export interface CanvasEdgeData extends Record<string, unknown> {
  afterNodeId: string
  beforeNodeId: string
  conditionBranch?: 'true' | 'false'
  insideBranch: boolean
}

/** Node ids that live inside some loop's body (only ids that still exist). */
export function loopBodyMembers(nodes: WorkflowNode[]): Set<string> {
  const ids = new Set(nodes.map((n) => n.id))
  const members = new Set<string>()
  for (const node of nodes) {
    if (node.type !== 'loop') continue
    for (const id of (node.config as LoopConfig).bodyNodeIds ?? []) {
      if (ids.has(id)) members.add(id)
    }
  }
  return members
}

/** The loop that owns a body node, if any. */
function owningLoopId(nodes: WorkflowNode[], bodyNodeId: string): string | undefined {
  return nodes.find(
    (n) => n.type === 'loop' && ((n.config as LoopConfig).bodyNodeIds ?? []).includes(bodyNodeId)
  )?.id
}

/** Estimated card height; only feeds the layout, so drift just widens a gap. */
export function estimateNodeHeight(node: WorkflowNode, allNodes: WorkflowNode[]): number {
  if (node.type === 'loop') {
    const body = ((node.config as LoopConfig).bodyNodeIds ?? [])
      .map((id) => allNodes.find((n) => n.id === id))
      .filter((n): n is WorkflowNode => !!n)
    const bodyHeights =
      body.length === 0
        ? 52
        : body.reduce((sum, b) => sum + estimateNodeHeight(b, allNodes), 0) + (body.length - 1) * 18
    // header + padding + body + line + add button + footer
    return 41 + 16 + bodyHeights + 18 + 22 + 40
  }
  if (node.type === 'condition') {
    const cfg = node.config as { variable?: string }
    return cfg.variable ? 90 : 58
  }
  // A trigger card draws one subtitle line for every kind; its stepPreview
  // (cron/event) belongs to the run trace, not the card.
  if (node.type === 'trigger') return 58
  return stepPreview(node) ? 90 : 58
}

interface Placed {
  positions: Map<string, { x: number; y: number }>
  /** Node ids drawn inside a fork branch, where loop/parallel insertion is off. */
  branchMembers: Set<string>
}

/** Positions from the FlowRow tree: vertical trunk, branches side by side, loops as one block. */
export function layoutPositions(nodes: WorkflowNode[], edges: WorkflowEdge[]): Placed {
  const rows = computeFlowLayout(nodes, edges)
  const positions = new Map<string, { x: number; y: number }>()
  const branchMembers = new Set<string>()
  const bodySet = loopBodyMembers(nodes)

  const rowWidth = (row: FlowRow): number => {
    if (row.kind === 'loop') return LOOP_WIDTH
    if (row.kind === 'node') return CARD_WIDTH
    const widths = row.branches.map(branchWidth)
    return widths.reduce((a, b) => a + b, 0) + (widths.length - 1) * BRANCH_GAP
  }
  const branchWidth = (branch: FlowRow[]): number =>
    branch.length === 0 ? CARD_WIDTH : Math.max(...branch.map(rowWidth))

  const place = (rows: FlowRow[], xCenter: number, y: number, insideBranch: boolean): number => {
    let cursor = y
    for (const row of rows) {
      if (row.kind === 'node') {
        // Orphaned body members draw inside their loop, not on the trunk.
        if (bodySet.has(row.node.id)) continue
        positions.set(row.node.id, { x: xCenter - CARD_WIDTH / 2, y: cursor })
        if (insideBranch) branchMembers.add(row.node.id)
        cursor += estimateNodeHeight(row.node, nodes) + ROW_GAP
      } else if (row.kind === 'loop') {
        positions.set(row.loopNode.id, { x: xCenter - LOOP_WIDTH / 2, y: cursor })
        if (insideBranch) branchMembers.add(row.loopNode.id)
        cursor += estimateNodeHeight(row.loopNode, nodes) + ROW_GAP
      } else {
        const widths = row.branches.map(branchWidth)
        const total = widths.reduce((a, b) => a + b, 0) + (widths.length - 1) * BRANCH_GAP
        let left = xCenter - total / 2
        let deepest = cursor
        row.branches.forEach((branch, i) => {
          const center = left + widths[i] / 2
          const bottom = place(branch, center, cursor, true)
          deepest = Math.max(deepest, bottom)
          left += widths[i] + BRANCH_GAP
        })
        cursor = deepest
      }
    }
    return cursor
  }

  place(rows, 0, 0, false)
  return { positions, branchMembers }
}

/** Whether every stored position is the untouched seed column (all x = 0). */
export function positionsAreSeed(nodes: WorkflowNode[]): boolean {
  return nodes.every((n) => !n.position || n.position.x === 0)
}

export interface CanvasElements {
  nodes: Node[]
  edges: Edge[]
  branchMembers: Set<string>
}

/** Stored positions when someone has arranged the workflow, the layout walk otherwise. */
export function toCanvasElements(nodes: WorkflowNode[], edges: WorkflowEdge[]): CanvasElements {
  const bodySet = loopBodyMembers(nodes)
  const { positions: computed, branchMembers } = layoutPositions(nodes, edges)
  const useComputed = positionsAreSeed(nodes)

  const rfNodes: Node[] = []
  for (const node of nodes) {
    if (bodySet.has(node.id)) continue
    const position = useComputed
      ? (computed.get(node.id) ?? { x: 0, y: 0 })
      : { x: node.position?.x ?? 0, y: node.position?.y ?? 0 }
    const width = node.type === 'loop' ? LOOP_WIDTH : CARD_WIDTH
    const height = estimateNodeHeight(node, nodes)
    rfNodes.push({
      id: node.id,
      type: node.type === 'loop' ? 'loop' : 'step',
      position,
      data: { nodeId: node.id } satisfies CanvasNodeData,
      // Initial dimensions and handles anchor edges before (and without) a DOM
      // measure; once mounted, measured card bounds take over.
      initialWidth: width,
      initialHeight: height,
      handles: [
        ...(node.type !== 'trigger'
          ? [{ type: 'target' as const, position: Position.Top, x: width / 2, y: 0 }]
          : []),
        { type: 'source' as const, position: Position.Bottom, x: width / 2, y: height }
      ]
    })
  }

  const rfEdges: Edge[] = []
  const seenEdgeIds = new Set<string>()
  for (const edge of edges) {
    const sourceInBody = bodySet.has(edge.source)
    const targetInBody = bodySet.has(edge.target)
    if (sourceInBody && targetInBody) continue
    if (!sourceInBody && targetInBody) continue
    if (seenEdgeIds.has(edge.id)) continue
    seenEdgeIds.add(edge.id)

    // Edges leaving a loop body draw from the composite; real endpoints stay for splicing.
    const drawnSource = sourceInBody ? owningLoopId(nodes, edge.source) : edge.source
    if (!drawnSource) continue

    rfEdges.push({
      id: edge.id,
      source: drawnSource,
      target: edge.target,
      type: 'step',
      label:
        edge.conditionBranch === 'true'
          ? 'True'
          : edge.conditionBranch === 'false'
            ? 'False'
            : undefined,
      data: {
        afterNodeId: edge.source,
        beforeNodeId: edge.target,
        conditionBranch: edge.conditionBranch,
        insideBranch:
          branchMembers.has(edge.source) ||
          branchMembers.has(edge.target) ||
          edge.conditionBranch !== undefined
      } satisfies CanvasEdgeData
    })
  }

  // Leaves are judged on the drawn graph: a terminal loop's body edges don't count.
  const hasOutgoing = new Set(rfEdges.map((e) => e.source))
  for (const node of nodes) {
    if (bodySet.has(node.id)) continue
    if (hasOutgoing.has(node.id)) continue
    const anchor = rfNodes.find((n) => n.id === node.id)
    if (!anchor) continue
    const width = node.type === 'loop' ? LOOP_WIDTH : CARD_WIDTH
    rfNodes.push({
      id: `add:${node.id}`,
      type: 'addStep',
      position: {
        x: anchor.position.x + width / 2 - 11,
        y: anchor.position.y + estimateNodeHeight(node, nodes) + 18
      },
      data: {
        afterNodeId: node.id,
        insideBranch: branchMembers.has(node.id)
      } satisfies AddStepNodeData,
      initialWidth: 22,
      initialHeight: 22,
      handles: [{ type: 'target' as const, position: Position.Top, x: 11, y: 0 }],
      draggable: false,
      selectable: false,
      // Its menu must open above neighbouring cards, and selection elevates those to 1000.
      zIndex: 1200
    })
    rfEdges.push({
      id: `add-edge:${node.id}`,
      source: node.id,
      target: `add:${node.id}`,
      type: 'step',
      selectable: false
    })
  }

  // A workflow with no trigger yet shows the spot where one goes, sitting
  // above the topmost drawn card and centered on it.
  if (!nodes.some((n) => n.type === 'trigger')) {
    const cards = rfNodes.filter((n) => n.type === 'step' || n.type === 'loop')
    let position = { x: 0, y: 0 }
    if (cards.length > 0) {
      const top = cards.reduce((a, b) => (b.position.y < a.position.y ? b : a))
      const topWidth = top.type === 'loop' ? LOOP_WIDTH : CARD_WIDTH
      position = {
        x: top.position.x + topWidth / 2 - CARD_WIDTH / 2,
        y: top.position.y - 58 - ROW_GAP
      }
    }
    rfNodes.push({
      id: 'add-trigger',
      type: 'addTrigger',
      position,
      data: {},
      width: CARD_WIDTH,
      height: 58,
      draggable: false,
      selectable: false
    })
  }

  return { nodes: rfNodes, edges: rfEdges, branchMembers }
}

/** Whether a hand-drawn source → target edge keeps the graph a DAG the engine understands. */
export function canConnect(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  source: string,
  target: string
): boolean {
  if (source === target) return false
  const sourceNode = nodes.find((n) => n.id === source)
  const targetNode = nodes.find((n) => n.id === target)
  if (!sourceNode || !targetNode) return false
  if (targetNode.type === 'trigger') return false
  // Condition branches carry managed tags; hand-drawn edges would bypass them.
  if (sourceNode.type === 'condition') return false
  const bodySet = loopBodyMembers(nodes)
  if (bodySet.has(source) || bodySet.has(target)) return false
  if (edges.some((e) => e.source === source && e.target === target)) return false
  const successors = new Map<string, string[]>()
  for (const e of edges) {
    const list = successors.get(e.source) ?? []
    list.push(e.target)
    successors.set(e.source, list)
  }
  const queue = [target]
  const seen = new Set<string>()
  while (queue.length > 0) {
    const current = queue.shift()!
    if (current === source) return false
    if (seen.has(current)) continue
    seen.add(current)
    queue.push(...(successors.get(current) ?? []))
  }
  return true
}

import { Position, type Edge, type Node } from '@xyflow/react'
import { LoopConfig, WorkflowEdge, WorkflowNode } from '../../shared/types'
import { stepPreview } from '../components/workflow-editor/node-visuals'
import { computeFlowLayout, FlowRow } from './workflow-helpers'

/**
 * Everything the canvas derives from a workflow definition: which definition
 * nodes become canvas nodes, where they sit, and which edges are drawn.
 *
 * The definition stays the source of truth. A loop and its body render as one
 * composite canvas node — membership is the loop's own `bodyNodeIds`, so body
 * steps never appear as free canvas nodes and an edge that leaves the body is
 * drawn from the composite. The layout walks the same `FlowRow` tree the rail
 * drew, so Tidy up reproduces exactly the order the rail showed.
 */

export const CARD_WIDTH = 280
export const LOOP_WIDTH = 312
/** Horizontal gap between fork branches. */
const BRANCH_GAP = 56
/** Vertical gap between consecutive steps (room for the edge). */
const ROW_GAP = 56

/** Data every canvas step/loop node carries: only the definition node's id.
 *  Content, selection, and status come from context so the node array is
 *  stable across selection and run-status changes. */
export interface CanvasNodeData extends Record<string, unknown> {
  nodeId: string
}

/** The + that trails every leaf, and what an insertion there means. */
export interface AddStepNodeData extends Record<string, unknown> {
  afterNodeId: string
  insideBranch: boolean
}

export interface CanvasEdgeData extends Record<string, unknown> {
  /** Arguments for the editor's insert path when the edge's + is used. */
  afterNodeId: string
  beforeNodeId: string
  conditionBranch?: 'true' | 'false'
  /** Loop and parallel insertion stay off edges inside a fork branch. */
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

/**
 * Rendered height of a step card, from the same facts the card renders:
 * a header is two lines, a preview adds a footer. Only feeds the layout —
 * a few points of drift just widens a gap.
 */
export function estimateNodeHeight(node: WorkflowNode, allNodes: WorkflowNode[]): number {
  if (node.type === 'loop') {
    const body = ((node.config as LoopConfig).bodyNodeIds ?? [])
      .map((id) => allNodes.find((n) => n.id === id))
      .filter((n): n is WorkflowNode => !!n)
    const bodyHeights =
      body.length === 0
        ? 52 // the "no steps yet" placeholder
        : body.reduce((sum, b) => sum + estimateNodeHeight(b, allNodes), 0) + (body.length - 1) * 18
    // header + top padding + body + line + add button + footer
    return 41 + 16 + bodyHeights + 18 + 22 + 40
  }
  return stepPreview(node) ? 90 : 58
}

interface Placed {
  positions: Map<string, { x: number; y: number }>
  /** Node ids drawn inside a fork branch — loop/parallel insertion stays off there. */
  branchMembers: Set<string>
}

/**
 * Positions for every top-level canvas node, derived from the FlowRow tree:
 * a vertical trunk, fork branches side by side, a loop as one tall block.
 * `xCenter` is the trunk's centerline in canvas coordinates.
 */
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
        // Body members can appear as appended orphans; they draw inside their
        // loop, so they take no space on the trunk.
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

/**
 * Project the definition into canvas elements. Positions come from the
 * definition when someone has arranged it, and from the layout walk when the
 * stored positions are still the seeded single column.
 */
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
      // Explicit dimensions and declared handles let edges render before the
      // first DOM measure — and at all in environments with no layout (tests).
      width,
      height,
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
    if (!sourceInBody && targetInBody) continue // the loop draws its own entry
    if (seenEdgeIds.has(edge.id)) continue
    seenEdgeIds.add(edge.id)

    // An edge that leaves a loop body continues the trunk: draw it from the
    // composite, but keep the real endpoints so insertion splices correctly.
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

  // Every leaf gets a trailing +, matching the rail's end-of-chain button.
  const hasOutgoing = new Set(edges.map((e) => e.source))
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
        y: anchor.position.y + estimateNodeHeight(node, nodes) + 26
      },
      data: {
        afterNodeId: node.id,
        insideBranch: branchMembers.has(node.id)
      } satisfies AddStepNodeData,
      width: 22,
      height: 22,
      handles: [{ type: 'target' as const, position: Position.Top, x: 11, y: 0 }],
      draggable: false,
      selectable: false
    })
    rfEdges.push({
      id: `add-edge:${node.id}`,
      source: node.id,
      target: `add:${node.id}`,
      type: 'step',
      style: { strokeDasharray: '4 4' },
      selectable: false
    })
  }

  return { nodes: rfNodes, edges: rfEdges, branchMembers }
}

/**
 * Would connecting `source` → `target` keep the definition a DAG with the
 * shapes the engine understands? Used to validate hand-drawn connections.
 */
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
  // A condition's branches carry true/false tags the panel manages; a loop
  // drives its own body. Hand-drawn edges from either would bypass that.
  if (sourceNode.type === 'condition') return false
  const bodySet = loopBodyMembers(nodes)
  if (bodySet.has(source) || bodySet.has(target)) return false
  if (edges.some((e) => e.source === source && e.target === target)) return false
  // No cycles: if source is reachable from target, this edge closes a loop.
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

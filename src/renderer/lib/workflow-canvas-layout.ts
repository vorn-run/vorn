import {
  getBezierPath,
  Position,
  type Edge,
  type Node,
  type Rect,
  type Viewport
} from '@xyflow/react'
import { WorkflowEdge, WorkflowNode, WorkflowNodePosition } from '../../shared/types'
import { collapseLoopBodies, loopBodyGraph, loopBodyOwners } from '@vornrun/shared/workflow-graph'
import { stepPreview } from '../components/workflow-editor/node-visuals'
import {
  CARD_WIDTH,
  computeFlowLayout,
  FlowRow,
  LOOP_WIDTH,
  snapToLattice
} from './workflow-helpers'

// Projects a workflow definition into canvas elements; the definition stays the source of truth.

/** The anchor id that opens the library in trigger scope. */
export const TRIGGER_ANCHOR_ID = '__TRIGGER__'

export const TRIGGER_ANCHOR = {
  afterNodeId: TRIGGER_ANCHOR_ID,
  beforeNodeId: null,
  insideBranch: false,
  bodyOnly: false
}

// Defined beside the placement that also uses them; re-exported here because
// this is where the canvas reaches for its geometry. The other direction would
// be a cycle: the layout walk itself comes from the helpers.
export { CARD_WIDTH, GRID, LOOP_WIDTH, snapToLattice } from './workflow-helpers'
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
  /** The loop whose body this edge runs inside, so a step inserted on it joins that body. */
  bodyOf?: string
}

/** The source handle inside a loop's frame, where each pass starts. */
export const LOOP_BODY_HANDLE = 'body'

/** Node ids that live inside some loop's body (only ids that still exist). */
export function loopBodyMembers(nodes: WorkflowNode[]): Set<string> {
  return new Set(loopBodyOwners(nodes).keys())
}

/** A loop frame's parts, top to bottom: header, padding above the body, then line, +, footer. */
const LOOP_HEADER = 41
const LOOP_BODY_PAD = 16
const LOOP_FOOTER = 18 + 22 + 40
/** The dashed "no steps yet" box an empty body shows. */
const EMPTY_BODY = 52

export interface LoopBodyLayout {
  /** Where each body step sits, relative to the loop's frame. */
  positions: Map<string, { x: number; y: number }>
  /** Body steps drawn inside a fork branch. */
  branchMembers: Set<string>
  /** The frame's width: the loop's own, or wider when its body branches. */
  width: number
  /** How tall the body is, not counting the frame around it. */
  height: number
}

/**
 * A loop's body laid out like a workflow of its own, inside the loop's frame.
 *
 * The same walk as the trunk, so a condition inside a loop forks the way one
 * outside does, and the frame grows to fit the widest branch.
 */
export function layoutLoopBody(
  loop: WorkflowNode,
  nodes: WorkflowNode[],
  edges: WorkflowEdge[]
): LoopBodyLayout {
  const { members, edges: inner } = loopBodyGraph(nodes, edges, loop)
  if (members.length === 0) {
    return { positions: new Map(), branchMembers: new Set(), width: LOOP_WIDTH, height: EMPTY_BODY }
  }
  const { positions, branchMembers } = layoutPositions(members, inner)
  let minX = Infinity
  let maxX = -Infinity
  let bottom = 0
  for (const member of members) {
    const at = positions.get(member.id)
    if (!at) continue
    minX = Math.min(minX, at.x)
    maxX = Math.max(maxX, at.x + CARD_WIDTH)
    bottom = Math.max(bottom, at.y + estimateNodeHeight(member, members))
  }
  const span = maxX - minX
  const width = Math.max(LOOP_WIDTH, span + 2 * LOOP_BODY_PAD)
  const shift = (width - span) / 2 - minX
  const relative = new Map<string, { x: number; y: number }>()
  for (const [id, at] of positions) {
    relative.set(id, {
      x: snapToLattice(at.x + shift),
      y: snapToLattice(at.y + LOOP_HEADER + LOOP_BODY_PAD)
    })
  }
  return { positions: relative, branchMembers, width, height: bottom }
}

/** Where a loop's body starts, measured from the top of its frame. */
export const LOOP_BODY_TOP = LOOP_HEADER + LOOP_BODY_PAD

/** Estimated card height; only feeds the layout, so drift just widens a gap. */
export function estimateNodeHeight(
  node: WorkflowNode,
  allNodes: WorkflowNode[],
  edges: WorkflowEdge[] = []
): number {
  if (node.type === 'loop') {
    return LOOP_HEADER + LOOP_BODY_PAD + layoutLoopBody(node, allNodes, edges).height + LOOP_FOOTER
  }
  if (node.type === 'condition') {
    const cfg = node.config as { variable?: string }
    return cfg.variable ? 90 : 58
  }
  return stepPreview(node) ? 90 : 58
}

interface Placed {
  positions: Map<string, { x: number; y: number }>
  /** Node ids drawn inside a fork branch, where loop/parallel insertion is off. */
  branchMembers: Set<string>
}

/** Positions from the FlowRow tree: vertical trunk, branches side by side, loops as one block. */
export function layoutPositions(nodes: WorkflowNode[], edges: WorkflowEdge[]): Placed {
  // The trunk walks past each loop to what follows it; its body is laid out inside it.
  const rows = computeFlowLayout(nodes, collapseLoopBodies(nodes, edges))
  const positions = new Map<string, { x: number; y: number }>()
  const branchMembers = new Set<string>()
  const bodySet = loopBodyMembers(nodes)

  const rowWidth = (row: FlowRow): number => {
    if (row.kind === 'loop') return layoutLoopBody(row.loopNode, nodes, edges).width
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
        positions.set(row.node.id, {
          x: snapToLattice(xCenter - CARD_WIDTH / 2),
          y: snapToLattice(cursor)
        })
        if (insideBranch) branchMembers.add(row.node.id)
        cursor += estimateNodeHeight(row.node, nodes) + ROW_GAP
      } else if (row.kind === 'loop') {
        positions.set(row.loopNode.id, {
          x: snapToLattice(xCenter - layoutLoopBody(row.loopNode, nodes, edges).width / 2),
          y: snapToLattice(cursor)
        })
        if (insideBranch) branchMembers.add(row.loopNode.id)
        cursor += estimateNodeHeight(row.loopNode, nodes, edges) + ROW_GAP
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

/**
 * A stored position, brought onto the lattice.
 *
 * Workflows arranged before the layout used the grid sit half a step off it, so
 * dragging one card snapped it 4px away from neighbours nobody touched and the
 * chain kinked. Healing on the way in costs at most 4px of drift from where a
 * card was left, and buys back a column that survives the next drag.
 */
export function latticePosition(node: WorkflowNode): WorkflowNodePosition {
  return {
    x: snapToLattice(node.position?.x ?? 0),
    y: snapToLattice(node.position?.y ?? 0)
  }
}

/** Stored positions that are already on the lattice, so a save can skip the write. */
export function positionsAreAligned(nodes: WorkflowNode[]): boolean {
  return nodes.every((node) => {
    const lattice = latticePosition(node)
    return lattice.x === (node.position?.x ?? 0) && lattice.y === (node.position?.y ?? 0)
  })
}

/** Every node with its position healed onto the lattice, for the next save. */
export function alignedNodes(nodes: WorkflowNode[]): WorkflowNode[] {
  if (positionsAreSeed(nodes) || positionsAreAligned(nodes)) return nodes
  return nodes.map((node) => ({ ...node, position: latticePosition(node) }))
}

export interface CanvasElements {
  nodes: Node[]
  edges: Edge[]
  branchMembers: Set<string>
}

/** Stored positions when someone has arranged the workflow, the layout walk otherwise. */
export function toCanvasElements(nodes: WorkflowNode[], edges: WorkflowEdge[]): CanvasElements {
  const owners = loopBodyOwners(nodes)
  const { positions: computed, branchMembers } = layoutPositions(nodes, edges)
  const useComputed = positionsAreSeed(nodes)
  const byId = new Map(nodes.map((n) => [n.id, n]))
  const widths = new Map<string, number>()

  const stepHandles = (node: WorkflowNode, width: number, height: number) => [
    ...(node.type !== 'trigger'
      ? [{ type: 'target' as const, position: Position.Top, x: width / 2, y: 0 }]
      : []),
    { type: 'source' as const, position: Position.Bottom, x: width / 2, y: height }
  ]

  const rfNodes: Node[] = []
  for (const node of nodes) {
    if (owners.has(node.id)) continue
    const position = useComputed ? (computed.get(node.id) ?? { x: 0, y: 0 }) : latticePosition(node)
    const body = node.type === 'loop' ? layoutLoopBody(node, nodes, edges) : undefined
    const width = body?.width ?? CARD_WIDTH
    const height = estimateNodeHeight(node, nodes, edges)
    widths.set(node.id, width)
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
        ...stepHandles(node, width, height),
        ...(body
          ? [
              {
                id: LOOP_BODY_HANDLE,
                type: 'source' as const,
                position: Position.Bottom,
                x: width / 2,
                y: LOOP_HEADER
              }
            ]
          : [])
      ]
    })
    if (!body) continue
    for (const branchMember of body.branchMembers) branchMembers.add(branchMember)
    // Children come right after their loop: React Flow draws a child only
    // once its parent is known. They sit where the body's own layout puts
    // them, relative to the frame, and move with it; they are never dragged.
    for (const [id, at] of body.positions) {
      const member = byId.get(id)
      if (!member) continue
      const memberHeight = estimateNodeHeight(member, nodes, edges)
      rfNodes.push({
        id,
        type: 'step',
        parentId: node.id,
        position: at,
        draggable: false,
        data: { nodeId: id } satisfies CanvasNodeData,
        initialWidth: CARD_WIDTH,
        initialHeight: memberHeight,
        handles: stepHandles(member, CARD_WIDTH, memberHeight)
      })
    }
  }

  const branchLabel = (edge: WorkflowEdge) =>
    edge.conditionBranch === 'true'
      ? 'True'
      : edge.conditionBranch === 'false'
        ? 'False'
        : undefined

  const rfEdges: Edge[] = []
  const seenEdgeIds = new Set<string>()
  // Two exits of a branching body leave the loop as one line to the step after it.
  const exitsTo = new Map<string, WorkflowEdge[]>()
  for (const edge of edges) {
    const sourceOwner = owners.get(edge.source)
    const targetOwner = owners.get(edge.target)
    if (sourceOwner && !targetOwner) {
      const key = `${sourceOwner}->${edge.target}`
      exitsTo.set(key, [...(exitsTo.get(key) ?? []), edge])
    }
  }

  for (const edge of edges) {
    if (seenEdgeIds.has(edge.id)) continue
    seenEdgeIds.add(edge.id)
    const sourceOwner = owners.get(edge.source)
    const targetOwner = owners.get(edge.target)

    if (targetOwner) {
      // Into a body: from its own loop (where each pass starts) or between its
      // own steps. Anything else feeding a body is refused elsewhere, not drawn.
      const entry = edge.source === targetOwner
      if (!entry && sourceOwner !== targetOwner) continue
      rfEdges.push({
        id: edge.id,
        source: edge.source,
        ...(entry && { sourceHandle: LOOP_BODY_HANDLE }),
        target: edge.target,
        type: 'step',
        label: branchLabel(edge),
        data: {
          afterNodeId: edge.source,
          beforeNodeId: edge.target,
          conditionBranch: edge.conditionBranch,
          insideBranch: edge.conditionBranch !== undefined || branchMembers.has(edge.target),
          bodyOf: targetOwner
        } satisfies CanvasEdgeData
      })
      continue
    }

    // Leaving a body: drawn from the loop, since the loop is what the next step waits on.
    if (sourceOwner) {
      const key = `${sourceOwner}->${edge.target}`
      const exits = exitsTo.get(key) ?? []
      if (exits[0] !== edge) continue
      rfEdges.push({
        id: edge.id,
        source: sourceOwner,
        target: edge.target,
        type: 'step',
        // One exit splices cleanly; several have no single place a step would go.
        data:
          exits.length === 1
            ? ({
                afterNodeId: edge.source,
                beforeNodeId: edge.target,
                insideBranch: branchMembers.has(sourceOwner) || branchMembers.has(edge.target)
              } satisfies CanvasEdgeData)
            : undefined
      })
      continue
    }

    rfEdges.push({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      type: 'step',
      label: branchLabel(edge),
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

  // Leaves are judged on the drawn trunk: a loop's own body does not make it lead anywhere.
  const hasOutgoing = new Set(
    rfEdges.filter((e) => !(e.data as CanvasEdgeData | undefined)?.bodyOf).map((e) => e.source)
  )
  for (const node of nodes) {
    if (owners.has(node.id)) continue
    if (hasOutgoing.has(node.id)) continue
    const anchor = rfNodes.find((n) => n.id === node.id)
    if (!anchor) continue
    const width = widths.get(node.id) ?? CARD_WIDTH
    // Deliberately off the lattice: this one is centred on the card rather than
    // snapped to it. Nobody can drag it, so nothing will knock it off, and its
    // port lands exactly under the card's — which a 4px rounding would lean.
    rfNodes.push({
      id: `add:${node.id}`,
      type: 'addStep',
      position: {
        x: anchor.position.x + width / 2 - 11,
        y: anchor.position.y + estimateNodeHeight(node, nodes, edges) + 18
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
    const cards = rfNodes.filter((n) => (n.type === 'step' || n.type === 'loop') && !n.parentId)
    let position = { x: 0, y: 0 }
    if (cards.length > 0) {
      const top = cards.reduce((a, b) => (b.position.y < a.position.y ? b : a))
      const topWidth = widths.get(top.id) ?? CARD_WIDTH
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

/**
 * Below this, two cards are meant to be in one column and a curve is a kink.
 *
 * A drag snaps to the 8px lattice, so a card can land a step off its parent
 * without anyone aiming for that. A bezier answers even a single step with an
 * S-bend, which reads as deliberate; a straight run reads as the near-miss it is.
 */
export const STRAIGHT_EDGE_TOLERANCE = 16

/** The path an edge draws: straight down a column, curved only for a real fork. */
export function stepEdgePath(params: {
  sourceX: number
  sourceY: number
  targetX: number
  targetY: number
  sourcePosition: Position
  targetPosition: Position
}): [string, number, number] {
  const { sourceX, sourceY, targetX, targetY } = params
  if (Math.abs(sourceX - targetX) < STRAIGHT_EDGE_TOLERANCE) {
    // Drawn between the ports themselves rather than down a shared centre, so
    // a slightly offset card keeps its line attached at both ends.
    return [
      `M ${sourceX},${sourceY} L ${targetX},${targetY}`,
      (sourceX + targetX) / 2,
      (sourceY + targetY) / 2
    ]
  }
  const [path, labelX, labelY] = getBezierPath(params)
  return [path, labelX, labelY]
}

/** Whether a hand-drawn source → target edge keeps the graph a DAG the engine understands. */
export function canConnect(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  source: string,
  target: string,
  sourceHandle?: string | null
): boolean {
  if (source === target) return false
  const sourceNode = nodes.find((n) => n.id === source)
  const targetNode = nodes.find((n) => n.id === target)
  if (!sourceNode || !targetNode) return false
  if (targetNode.type === 'trigger') return false
  // Condition branches carry managed tags; hand-drawn edges would bypass them.
  if (sourceNode.type === 'condition') return false
  // A body is fed only by its own loop, from the handle inside its frame, or
  // by its own steps; a loop's body handle feeds nothing else.
  const owners = loopBodyOwners(nodes)
  const targetOwner = owners.get(target)
  if (sourceHandle === LOOP_BODY_HANDLE) {
    if (targetOwner !== source) return false
  } else if (targetOwner && owners.get(source) !== targetOwner) {
    return false
  }
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

/** How far the first step sits below the top of the canvas when a workflow opens. */
const OPENING_TOP = 48

type PlacedNode = Pick<Node, 'position' | 'type' | 'width' | 'measured'>

/** The add buttons drawn around the steps, which are not steps themselves. */
export function isPlaceholder(node: Pick<Node, 'type'>): boolean {
  return node.type === 'addStep' || node.type === 'addTrigger'
}

/** Where a workflow first opens: 100%, its steps centred and the first near the top. */
export function openingViewport(nodes: PlacedNode[], width: number): Viewport {
  const steps = nodes.filter((n) => !isPlaceholder(n))
  const drawn = steps.length > 0 ? steps : nodes
  const widthOf = (n: PlacedNode): number =>
    n.measured?.width ?? n.width ?? (n.type === 'loop' ? LOOP_WIDTH : CARD_WIDTH)
  const minX = Math.min(...drawn.map((n) => n.position.x))
  const maxX = Math.max(...drawn.map((n) => n.position.x + widthOf(n)))
  const minY = Math.min(...drawn.map((n) => n.position.y))
  return placeAtTop({ x: minX, y: minY, width: maxX - minX, height: 0 }, width, 1)
}

/** The furthest the canvas zooms out. */
export const CANVAS_MIN_ZOOM = 0.2

/** The whole workflow on screen, never past 100%, with its first step near the top rather than centred. */
export function topAlignedFit(bounds: Rect, width: number, height: number): Viewport {
  const fits = Math.min((width * 0.9) / bounds.width, (height - 2 * OPENING_TOP) / bounds.height)
  return placeAtTop(bounds, width, Math.min(1, Math.max(CANVAS_MIN_ZOOM, fits)))
}

/** Centred across the canvas, with the top of the bounds just below its top edge. */
function placeAtTop(bounds: Rect, width: number, zoom: number): Viewport {
  return {
    x: width / 2 - (bounds.x + bounds.width / 2) * zoom,
    y: OPENING_TOP - bounds.y * zoom,
    zoom
  }
}

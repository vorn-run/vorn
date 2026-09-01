import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react'
import {
  Background,
  BackgroundVariant,
  BaseEdge,
  ControlButton,
  Controls,
  EdgeLabelRenderer,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  ReactFlowProvider,
  applyNodeChanges,
  getBezierPath,
  useReactFlow,
  useUpdateNodeInternals,
  type Connection,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeChange,
  type NodeProps
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { AlignVerticalSpaceAround, Repeat, Replace, StepForward, Trash2, Zap } from 'lucide-react'
import { LoopConfig, NodeExecutionStatus, WorkflowEdge, WorkflowNode } from '../../../shared/types'
import {
  AddStepNodeData,
  CanvasEdgeData,
  canConnect,
  estimateNodeHeight,
  toCanvasElements,
  TRIGGER_ANCHOR,
  TRIGGER_ANCHOR_ID
} from '../../lib/workflow-canvas-layout'
import { REPLACEABLE_NODE_TYPES } from '../../lib/workflow-helpers'
import { NODE_GLYPH, NODE_SELECTED, NODE_UNSELECTED } from './node-visuals'
import { WORKFLOW_STATUS_DOT_PULSE } from '../../lib/workflow-status'
import { Tooltip } from '../Tooltip'
import { NodeCard } from './nodes/NodeCard'
import { ConnectorButton } from './nodes/AddStepNode'

export type AddableNodeType =
  | 'agent'
  | 'script'
  | 'condition'
  | 'approval'
  | 'connectorAction'
  | 'httpRequest'
  | 'loop'

/** Where a pick from the step library lands, and what that spot allows. */
export interface InsertAnchor {
  afterNodeId: string
  /** A node id to splice before, `'__LOOP_BODY__'`, or null to append. */
  beforeNodeId: string | null
  insideBranch: boolean
  bodyOnly: boolean
  /** Flow coordinates for picks that land where something was dropped. */
  position?: { x: number; y: number }
  /** Set when the pick swaps this node in place instead of inserting. */
  replaceNodeId?: string
}

function isAnchor(anchor: InsertAnchor | null, afterNodeId: string, beforeNodeId: string | null) {
  return anchor?.afterNodeId === afterNodeId && anchor?.beforeNodeId === beforeNodeId
}

interface Props {
  nodes: WorkflowNode[]
  edges: WorkflowEdge[]
  onNodeClick: (nodeId: string) => void
  /** Every + routes here; the editor docks the step library for the anchor. */
  onOpenLibrary: (anchor: InsertAnchor) => void
  /** The anchor the open library points at; its + stays lit. */
  libraryAnchor: InsertAnchor | null
  /** A hand-drawn, validated connection between two existing steps. */
  onConnectEdge: (sourceId: string, targetId: string) => void
  /** Dragged nodes settled; write the new positions into the definition. */
  onPositionsCommit: (positions: Record<string, { x: number; y: number }>) => void
  /** Delete-key removal of the selected step, trigger included. */
  onDeleteNode?: (nodeId: string) => void
  /** Hover-toolbar shortcut into the editor's run-to-step path. */
  onRunToStep?: (nodeId: string) => void
  onTidyUp: () => void
  selectedNodeId: string | null
  /** What each node is doing in live runs; absent when nothing is running. */
  nodeStatus?: Record<string, NodeExecutionStatus>
  /** Changes when a different workflow loads; re-fits the view top-aligned. */
  loadKey?: string | null
}

/** Kept in context so selection/status churn re-renders cards without rebuilding the node array. */
interface CanvasInteractions {
  nodesById: Map<string, WorkflowNode>
  allNodes: WorkflowNode[]
  selectedNodeId: string | null
  nodeStatus?: Record<string, NodeExecutionStatus>
  onNodeClick: (nodeId: string) => void
  onOpenLibrary: (anchor: InsertAnchor) => void
  libraryAnchor: InsertAnchor | null
  onDeleteNode?: (nodeId: string) => void
  onRunToStep?: (nodeId: string) => void
}

const InteractionsContext = createContext<CanvasInteractions | null>(null)

function useInteractions(): CanvasInteractions {
  const ctx = useContext(InteractionsContext)
  if (!ctx) throw new Error('Canvas node rendered outside the workflow canvas')
  return ctx
}

/** The strip that floats over a hovered card; the wrapper must carry `group`. */
const TOOLBAR_BUTTON = `p-1.5 rounded-md bg-surface-overlay border border-white/[0.12] text-gray-400
                        hover:text-white hover:border-white/[0.2] transition-colors`

function NodeHoverToolbar({ nodeId }: { nodeId: string }) {
  const { nodesById, onDeleteNode, onRunToStep, onOpenLibrary } = useInteractions()
  const node = nodesById.get(nodeId)
  if (!onDeleteNode && !onRunToStep) return null
  const isTrigger = node?.type === 'trigger'
  const replaceable = !!node && REPLACEABLE_NODE_TYPES.has(node.type)
  return (
    <div
      className="absolute left-full top-1/2 -translate-y-1/2 ml-1.5 flex flex-col gap-1 opacity-0
                 group-hover:opacity-100 transition-opacity duration-100 z-10"
    >
      {onRunToStep && !isTrigger && (
        <Tooltip label="Run to this step" position="right">
          <button
            aria-label="Run to this step"
            onClick={(e) => {
              e.stopPropagation()
              onRunToStep(nodeId)
            }}
            className={TOOLBAR_BUTTON}
          >
            <StepForward size={12} />
          </button>
        </Tooltip>
      )}
      {(replaceable || isTrigger) && (
        <Tooltip label="Replace step" position="right">
          <button
            aria-label="Replace step"
            onClick={(e) => {
              e.stopPropagation()
              // The trigger swaps through its own library scope; both keep id and edges.
              onOpenLibrary(
                isTrigger
                  ? TRIGGER_ANCHOR
                  : {
                      afterNodeId: nodeId,
                      beforeNodeId: null,
                      insideBranch: false,
                      bodyOnly: false,
                      replaceNodeId: nodeId
                    }
              )
            }}
            className={TOOLBAR_BUTTON}
          >
            <Replace size={12} />
          </button>
        </Tooltip>
      )}
      {onDeleteNode && (
        <Tooltip label="Delete step" position="right">
          <button
            aria-label="Delete step"
            onClick={(e) => {
              e.stopPropagation()
              onDeleteNode(nodeId)
            }}
            className={TOOLBAR_BUTTON}
          >
            <Trash2 size={12} />
          </button>
        </Tooltip>
      )}
    </div>
  )
}

const HANDLE_CLASS = '!w-[7px] !h-[7px] !bg-surface-base !border !border-white/[0.35] !rounded-full'

/** A single step: the existing card, with ports above and below. */
function StepNode({ data, id }: NodeProps) {
  const { nodesById, allNodes, selectedNodeId, nodeStatus, onNodeClick } = useInteractions()
  const node = nodesById.get(data.nodeId as string)
  const updateNodeInternals = useUpdateNodeInternals()
  // A replace-in-place keeps the id, so React Flow would keep the old card's
  // measured handle positions; re-measure when the rendered shape changes.
  // Never on mount: that races the initial measure while fitView settles.
  // Trigger kinds share a type and height, so the kind is part of the shape.
  const kind = (node?.config as { triggerType?: string } | undefined)?.triggerType ?? ''
  const shape = node ? `${node.type}:${kind}:${estimateNodeHeight(node, allNodes)}` : ''
  const lastShape = useRef<string | null>(null)
  useEffect(() => {
    if (shape && lastShape.current !== null && lastShape.current !== shape) {
      updateNodeInternals(id)
    }
    lastShape.current = shape || lastShape.current
  }, [id, shape, updateNodeInternals])
  if (!node) return null

  return (
    <div className="relative group">
      {node.type !== 'trigger' && (
        <Handle type="target" position={Position.Top} className={HANDLE_CLASS} />
      )}
      <NodeHoverToolbar nodeId={node.id} />
      <NodeCard
        node={node}
        selected={node.id === selectedNodeId}
        onClick={() => onNodeClick(node.id)}
        executionStatus={nodeStatus?.[node.id]}
      />
      <Handle type="source" position={Position.Bottom} className={HANDLE_CLASS} />
    </div>
  )
}

/** A loop and its body as one enclosure; membership stays `bodyNodeIds`, not canvas geometry. */
function LoopNode({ data, id }: NodeProps) {
  const {
    nodesById,
    allNodes,
    selectedNodeId,
    nodeStatus,
    onNodeClick,
    onOpenLibrary,
    libraryAnchor
  } = useInteractions()
  const node = nodesById.get(data.nodeId as string)
  const updateNodeInternals = useUpdateNodeInternals()
  const shape = node ? `${node.type}:${estimateNodeHeight(node, allNodes)}` : ''
  const lastShape = useRef<string | null>(null)
  useEffect(() => {
    if (shape && lastShape.current !== null && lastShape.current !== shape) {
      updateNodeInternals(id)
    }
    lastShape.current = shape || lastShape.current
  }, [id, shape, updateNodeInternals])
  if (!node || node.type !== 'loop') return null

  const config = node.config as LoopConfig
  const selected = node.id === selectedNodeId
  const until = config.until?.variable
    ? `until ${config.until.variable} ${config.until.operator} ${config.until.value}`
    : 'runs every pass'
  const body = (config.bodyNodeIds ?? [])
    .map((id) => allNodes.find((n) => n.id === id))
    .filter((n): n is WorkflowNode => !!n)
  const lastBodyId = body.length > 0 ? body[body.length - 1].id : null
  const loopStatus = nodeStatus?.[node.id]
  const bodyAnchorAfter = lastBodyId ?? node.id

  return (
    <div className="relative group">
      <Handle type="target" position={Position.Top} className={HANDLE_CLASS} />
      <NodeHoverToolbar nodeId={node.id} />
      <div
        data-loop-rail
        className={`w-[312px] rounded-lg border transition-all relative
                    ${selected ? NODE_SELECTED : loopStatus === 'error' ? 'border-danger/60' : NODE_UNSELECTED}
                    bg-surface-node`}
      >
        {loopStatus === 'running' && (
          <span
            aria-hidden
            className="absolute inset-0 rounded-lg border border-white/[0.35] animate-pulse pointer-events-none"
          />
        )}
        <div
          onClick={(e) => {
            e.stopPropagation()
            onNodeClick(node.id)
          }}
          className="flex items-center gap-2 px-4 py-2.5 border-b border-white/[0.06] cursor-pointer
                     hover:bg-white/[0.02] rounded-t-lg"
        >
          <Repeat size={13} className={`shrink-0 ${NODE_GLYPH}`} strokeWidth={2} />
          <span className="text-[12.5px] font-semibold text-white truncate flex-1">
            {node.label}
          </span>
          <span className="shrink-0 text-[10px] font-mono text-gray-400 bg-white/[0.06] rounded px-1.5 py-0.5">
            max {config.maxIterations ?? 1}
          </span>
          {loopStatus && WORKFLOW_STATUS_DOT_PULSE[loopStatus] && (
            <span
              data-loop-status
              className={`shrink-0 w-1.5 h-1.5 rounded-full ${WORKFLOW_STATUS_DOT_PULSE[loopStatus]}`}
            />
          )}
        </div>

        <div className="px-4 pt-4 flex flex-col items-center">
          {body.length === 0 ? (
            <div
              className="w-full rounded-md border border-dashed border-white/[0.12] px-3 py-4
                         text-[11px] text-gray-500 text-center"
            >
              No steps yet — add one below
            </div>
          ) : (
            body.map((bodyNode, i) => (
              <div key={bodyNode.id} className="flex flex-col items-center">
                {i > 0 && <div className="w-px h-[18px] bg-white/[0.08]" />}
                <NodeCard
                  node={bodyNode}
                  selected={bodyNode.id === selectedNodeId}
                  onClick={() => onNodeClick(bodyNode.id)}
                  executionStatus={nodeStatus?.[bodyNode.id]}
                />
              </div>
            ))
          )}

          {/* Inside the rail, so position is what decides membership. */}
          <div className="w-px h-[18px] bg-white/[0.08]" />
          <ConnectorButton
            active={isAnchor(libraryAnchor, bodyAnchorAfter, '__LOOP_BODY__')}
            onOpen={() =>
              onOpenLibrary({
                afterNodeId: bodyAnchorAfter,
                beforeNodeId: '__LOOP_BODY__',
                insideBranch: false,
                bodyOnly: true
              })
            }
          />
        </div>

        <div className="px-4 pt-2.5 pb-3 text-[10px] font-mono text-gray-500 text-center truncate">
          ↻ {until}
        </div>
      </div>
      <Handle type="source" position={Position.Bottom} className={HANDLE_CLASS} />
    </div>
  )
}

/** The + that trails every leaf. */
function AddStepNode({ data }: NodeProps) {
  const { onOpenLibrary, libraryAnchor } = useInteractions()
  const { afterNodeId, insideBranch } = data as unknown as AddStepNodeData

  return (
    // The wrapper of an unselectable, undraggable node gets pointer-events none.
    <div className="relative pointer-events-auto">
      <Handle type="target" position={Position.Top} className="!opacity-0 !pointer-events-none" />
      <ConnectorButton
        active={isAnchor(libraryAnchor, afterNodeId, null)}
        onOpen={() =>
          onOpenLibrary({ afterNodeId, beforeNodeId: null, insideBranch, bodyOnly: false })
        }
      />
    </div>
  )
}

/** The dashed spot where a workflow's trigger goes; opens the library in trigger scope. */
function AddTriggerNode() {
  const { onOpenLibrary, libraryAnchor } = useInteractions()
  const active = libraryAnchor?.afterNodeId === TRIGGER_ANCHOR_ID
  return (
    <div className="relative pointer-events-auto">
      <button
        onClick={() => onOpenLibrary(TRIGGER_ANCHOR)}
        className={`w-[280px] h-[58px] flex items-center justify-center gap-2 rounded-lg border border-dashed
                    text-[13px] transition-colors
                    ${
                      active
                        ? 'border-white/40 text-white'
                        : 'border-white/[0.15] text-gray-500 hover:text-gray-300 hover:border-white/[0.3]'
                    }`}
      >
        <Zap size={14} strokeWidth={2} />
        Add a trigger
      </button>
    </div>
  )
}

/** The line, the branch pill, and a hover + whose delayed leave lets the mouse reach it. */
function StepEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  label,
  style,
  data
}: EdgeProps) {
  const { onOpenLibrary, libraryAnchor } = useInteractions()
  const [hovered, setHovered] = useState(false)
  const leaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    return () => {
      if (leaveTimer.current) clearTimeout(leaveTimer.current)
    }
  }, [])

  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition
  })

  const edgeData = data as CanvasEdgeData | undefined
  const insertable = !!edgeData?.afterNodeId && !!edgeData?.beforeNodeId
  const anchored =
    insertable && isAnchor(libraryAnchor, edgeData!.afterNodeId, edgeData!.beforeNodeId)

  const enter = () => {
    if (leaveTimer.current) clearTimeout(leaveTimer.current)
    setHovered(true)
  }
  const leave = () => {
    if (leaveTimer.current) clearTimeout(leaveTimer.current)
    leaveTimer.current = setTimeout(() => setHovered(false), 600)
  }

  return (
    <g onMouseEnter={enter} onMouseLeave={leave}>
      <BaseEdge
        id={id}
        path={path}
        interactionWidth={40}
        style={{ stroke: 'rgba(255,255,255,0.16)', strokeWidth: 1.5, ...style }}
      />
      <EdgeLabelRenderer>
        <div
          className="absolute flex flex-col items-center gap-1 pointer-events-auto"
          style={{
            transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
            // Hovered, the + must rise above the cards (selection elevates those to 1000).
            zIndex: hovered || anchored ? 1300 : 'auto'
          }}
          onMouseEnter={enter}
          onMouseLeave={leave}
        >
          {label ? (
            <div
              data-branch-label
              className="px-2.5 py-0.5 rounded-full text-[10px] font-semibold border border-white/[0.08] text-ink-secondary bg-surface-base"
            >
              {label}
            </div>
          ) : null}
          {insertable && (hovered || anchored) && (
            <ConnectorButton
              active={anchored}
              onOpen={() =>
                onOpenLibrary({
                  afterNodeId: edgeData!.afterNodeId,
                  beforeNodeId: edgeData!.beforeNodeId,
                  insideBranch: edgeData!.insideBranch,
                  bodyOnly: false
                })
              }
            />
          )}
        </div>
      </EdgeLabelRenderer>
    </g>
  )
}

const NODE_TYPES = {
  step: StepNode,
  loop: LoopNode,
  addStep: AddStepNode,
  addTrigger: AddTriggerNode
}
const EDGE_TYPES = { step: StepEdge }

function WorkflowCanvasInner({
  nodes,
  edges,
  onNodeClick,
  onOpenLibrary,
  libraryAnchor,
  onConnectEdge,
  onPositionsCommit,
  onDeleteNode,
  onRunToStep,
  onTidyUp,
  selectedNodeId,
  nodeStatus,
  loadKey
}: Props) {
  const { screenToFlowPosition, zoomIn, zoomOut, zoomTo, fitView, getViewport, setViewport } =
    useReactFlow()
  const wrapperRef = useRef<HTMLDivElement>(null)

  const elements = useMemo(() => toCanvasElements(nodes, edges), [nodes, edges])
  const [rfNodes, setRfNodes] = useState<Node[]>(elements.nodes)

  // Adjust-state-while-rendering: rebuild from the definition; drag positions stay local.
  const [syncedElements, setSyncedElements] = useState(elements)
  if (syncedElements !== elements) {
    setSyncedElements(elements)
    setRfNodes(elements.nodes)
  }

  // The flow is vertical: on load, keep fitView's zoom and centering but pin
  // the topmost node near the top instead of vertically centering the chain.
  const elementsRef = useRef(elements)
  useEffect(() => {
    elementsRef.current = elements
  }, [elements])
  const alignTopView = useCallback(async () => {
    const drawn = elementsRef.current.nodes
    if (drawn.length === 0) return
    await fitView({ padding: 0.2, maxZoom: 1 })
    const { x, zoom } = getViewport()
    const minY = Math.min(...drawn.map((n) => n.position.y))
    setViewport({ x, y: 48 - minY * zoom, zoom })
  }, [fitView, getViewport, setViewport])

  const [rfReady, setRfReady] = useState(false)
  useEffect(() => {
    if (!rfReady) return
    void alignTopView()
  }, [rfReady, loadKey, alignTopView])

  const handleNodesChange = useCallback((changes: NodeChange[]) => {
    // The canvas owns position only; selection and structure stay the editor's.
    const positional = changes.filter((c) => c.type === 'position' || c.type === 'dimensions')
    if (positional.length > 0) setRfNodes((prev) => applyNodeChanges(positional, prev))
  }, [])

  const handleDragStop = useCallback(() => {
    // Committing every displayed position materializes the computed layout on first drag.
    const positions: Record<string, { x: number; y: number }> = {}
    for (const rfNode of rfNodes) {
      if (rfNode.type === 'addStep' || rfNode.type === 'addTrigger') continue
      positions[rfNode.id] = { x: rfNode.position.x, y: rfNode.position.y }
    }
    onPositionsCommit(positions)
  }, [rfNodes, onPositionsCommit])

  const isValidConnection = useCallback(
    (connection: Connection | Edge) =>
      !!connection.source &&
      !!connection.target &&
      canConnect(nodes, edges, connection.source, connection.target),
    [nodes, edges]
  )

  const handleConnect = useCallback(
    (connection: Connection) => {
      if (connection.source && connection.target) {
        onConnectEdge(connection.source, connection.target)
      }
    },
    [onConnectEdge]
  )

  const openLibraryAppend = useCallback(
    (afterNodeId: string, position?: { x: number; y: number }) => {
      onOpenLibrary({
        afterNodeId,
        beforeNodeId: null,
        insideBranch: elements.branchMembers.has(afterNodeId),
        bodyOnly: false,
        position
      })
    },
    [onOpenLibrary, elements.branchMembers]
  )

  const pendingConnectSource = useRef<string | null>(null)

  const handleConnectStart = useCallback((_: unknown, params: { nodeId: string | null }) => {
    pendingConnectSource.current = params.nodeId
  }, [])

  const handleConnectEnd = useCallback(
    (event: MouseEvent | TouchEvent, connectionState: { isValid: boolean | null }) => {
      const source = pendingConnectSource.current
      pendingConnectSource.current = null
      // A drop on empty canvas opens the library, remembering where the edge was released.
      if (connectionState.isValid === null && source && 'clientX' in event) {
        openLibraryAppend(source, screenToFlowPosition({ x: event.clientX, y: event.clientY }))
      }
    },
    [openLibraryAppend, screenToFlowPosition]
  )

  const leafForTabInsert = useCallback((): string | null => {
    const hasOutgoing = new Set(edges.map((e) => e.source))
    const leaf = [...nodes].reverse().find((n) => !hasOutgoing.has(n.id))
    return leaf?.id ?? null
  }, [nodes, edges])

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const target = e.target as HTMLElement
      if (target.closest('input, textarea, [contenteditable]')) return
      if (e.key === 'Tab') {
        const afterNodeId = leafForTabInsert()
        if (!afterNodeId) return
        e.preventDefault()
        openLibraryAppend(afterNodeId)
      } else if (e.key === '+' || e.key === '=') {
        void zoomIn()
      } else if (e.key === '-') {
        void zoomOut()
      } else if (e.key === '0') {
        void zoomTo(1)
      } else if (e.key === '1') {
        void fitView({ padding: 0.2, maxZoom: 1 })
      } else if ((e.key === 'Delete' || e.key === 'Backspace') && selectedNodeId) {
        const node = nodes.find((n) => n.id === selectedNodeId)
        if (node && onDeleteNode) {
          e.preventDefault()
          onDeleteNode(selectedNodeId)
        }
      }
    },
    [
      leafForTabInsert,
      openLibraryAppend,
      zoomIn,
      zoomOut,
      zoomTo,
      fitView,
      selectedNodeId,
      nodes,
      onDeleteNode
    ]
  )

  const interactions = useMemo<CanvasInteractions>(
    () => ({
      nodesById: new Map(nodes.map((n) => [n.id, n])),
      allNodes: nodes,
      selectedNodeId,
      nodeStatus,
      onNodeClick,
      onOpenLibrary,
      libraryAnchor,
      onDeleteNode,
      onRunToStep
    }),
    [
      nodes,
      selectedNodeId,
      nodeStatus,
      onNodeClick,
      onOpenLibrary,
      libraryAnchor,
      onDeleteNode,
      onRunToStep
    ]
  )

  return (
    <InteractionsContext.Provider value={interactions}>
      <div
        ref={wrapperRef}
        className="flex-1 h-full relative outline-none"
        tabIndex={0}
        onKeyDown={handleKeyDown}
      >
        <ReactFlow
          nodes={rfNodes}
          edges={elements.edges}
          nodeTypes={NODE_TYPES}
          edgeTypes={EDGE_TYPES}
          onNodesChange={handleNodesChange}
          onNodeDragStop={handleDragStop}
          onConnect={handleConnect}
          onConnectStart={handleConnectStart}
          onConnectEnd={handleConnectEnd}
          isValidConnection={isValidConnection}
          onPaneClick={() => onNodeClick('')}
          onInit={() => setRfReady(true)}
          minZoom={0.2}
          maxZoom={1.75}
          snapToGrid
          snapGrid={[8, 8]}
          connectionRadius={60}
          panOnScroll
          deleteKeyCode={null}
          selectionKeyCode={null}
          multiSelectionKeyCode={null}
          nodesFocusable={false}
          edgesFocusable={false}
          colorMode="dark"
          style={{ background: 'var(--color-surface-base)' }}
          defaultEdgeOptions={{ type: 'step' }}
          proOptions={{ hideAttribution: true }}
        >
          <Background
            variant={BackgroundVariant.Dots}
            gap={20}
            size={1}
            color="rgba(255,255,255,0.05)"
            bgColor="var(--color-surface-base)"
          />
          <MiniMap
            pannable
            zoomable
            className="!bg-surface-panel !border !border-white/[0.12] !rounded"
            maskColor="rgba(0,0,0,0.55)"
            nodeColor="rgba(255,255,255,0.25)"
            style={{ width: 96, height: 64 }}
          />
          <Controls
            showInteractive={false}
            className="!bg-surface-overlay !border !border-white/[0.12] !rounded-md !shadow-none
                       [&_button]:!bg-transparent [&_button]:!border-white/[0.08] [&_button]:!fill-gray-400"
          >
            <ControlButton onClick={onTidyUp} title="Tidy up">
              <AlignVerticalSpaceAround size={12} className="!fill-none stroke-gray-400" />
            </ControlButton>
          </Controls>
        </ReactFlow>
      </div>
    </InteractionsContext.Provider>
  )
}

export function WorkflowCanvas(props: Props) {
  return (
    <ReactFlowProvider>
      <WorkflowCanvasInner {...props} />
    </ReactFlowProvider>
  )
}

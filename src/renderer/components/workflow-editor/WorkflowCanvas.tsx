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
  type Connection,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeChange,
  type NodeProps
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { AlignVerticalSpaceAround, Repeat } from 'lucide-react'
import { LoopConfig, NodeExecutionStatus, WorkflowEdge, WorkflowNode } from '../../../shared/types'
import {
  AddStepNodeData,
  CanvasEdgeData,
  canConnect,
  toCanvasElements
} from '../../lib/workflow-canvas-layout'
import { useConnections } from '../../lib/use-connections'
import { NODE_GLYPH, NODE_SELECTED, NODE_UNSELECTED } from './node-visuals'
import { WORKFLOW_STATUS_DOT_PULSE } from '../../lib/workflow-status'
import { NodeCard } from './nodes/NodeCard'
import { ConnectorButton } from './nodes/AddStepNode'
import { NodePalette, PaletteConnectorItem, PalettePick } from './panels/NodePalette'

export type AddableNodeType =
  | 'agent'
  | 'script'
  | 'condition'
  | 'approval'
  | 'connectorAction'
  | 'loop'

interface Props {
  nodes: WorkflowNode[]
  edges: WorkflowEdge[]
  onNodeClick: (nodeId: string) => void
  onInsertNode: (afterNodeId: string, beforeNodeId: string | null, type: AddableNodeType) => void
  onAddParallelBranch: (forkFromId: string, type: 'agent' | 'script') => void
  /** A hand-drawn, validated connection between two existing steps. */
  onConnectEdge: (sourceId: string, targetId: string) => void
  /** A palette pick, appended after `afterNodeId` at `position` (flow coords). */
  onPaletteInsert: (
    pick: PalettePick,
    afterNodeId: string,
    position: { x: number; y: number }
  ) => void
  /** Dragged nodes settled; write the new positions into the definition. */
  onPositionsCommit: (positions: Record<string, { x: number; y: number }>) => void
  /** Delete-key removal of the selected step (never the trigger). */
  onDeleteNode?: (nodeId: string) => void
  onTidyUp: () => void
  selectedNodeId: string | null
  /** What each node is doing in live runs; absent when nothing is running. */
  nodeStatus?: Record<string, NodeExecutionStatus>
}

/** Kept in context so selection/status churn re-renders cards without rebuilding the node array. */
interface CanvasInteractions {
  nodesById: Map<string, WorkflowNode>
  allNodes: WorkflowNode[]
  selectedNodeId: string | null
  nodeStatus?: Record<string, NodeExecutionStatus>
  onNodeClick: (nodeId: string) => void
  onInsertNode: (afterNodeId: string, beforeNodeId: string | null, type: AddableNodeType) => void
  onAddParallelBranch: (forkFromId: string, type: 'agent' | 'script') => void
}

const InteractionsContext = createContext<CanvasInteractions | null>(null)

function useInteractions(): CanvasInteractions {
  const ctx = useContext(InteractionsContext)
  if (!ctx) throw new Error('Canvas node rendered outside the workflow canvas')
  return ctx
}

const HANDLE_CLASS = '!w-[7px] !h-[7px] !bg-surface-base !border !border-white/[0.35] !rounded-full'

/** A single step: the existing card, with ports above and below. */
function StepNode({ data }: NodeProps) {
  const { nodesById, selectedNodeId, nodeStatus, onNodeClick } = useInteractions()
  const node = nodesById.get(data.nodeId as string)
  if (!node) return null

  return (
    <div className="relative">
      {node.type !== 'trigger' && (
        <Handle type="target" position={Position.Top} className={HANDLE_CLASS} />
      )}
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
function LoopNode({ data }: NodeProps) {
  const { nodesById, allNodes, selectedNodeId, nodeStatus, onNodeClick, onInsertNode } =
    useInteractions()
  const node = nodesById.get(data.nodeId as string)
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

  return (
    <div className="relative">
      <Handle type="target" position={Position.Top} className={HANDLE_CLASS} />
      <div
        data-loop-rail
        className={`w-[312px] rounded-lg border transition-all
                    ${selected ? NODE_SELECTED : NODE_UNSELECTED}
                    bg-surface-node`}
      >
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
            onAddAction={() => onInsertNode(lastBodyId ?? node.id, '__LOOP_BODY__', 'agent')}
            onAddScript={() => onInsertNode(lastBodyId ?? node.id, '__LOOP_BODY__', 'script')}
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

/** The + that trails every leaf, carrying the same menu the rail ended with. */
function AddStepNode({ data }: NodeProps) {
  const { onInsertNode, onAddParallelBranch } = useInteractions()
  const { afterNodeId, insideBranch } = data as unknown as AddStepNodeData

  return (
    // The wrapper of an unselectable, undraggable node gets pointer-events none.
    <div className="relative pointer-events-auto">
      <Handle type="target" position={Position.Top} className="!opacity-0 !pointer-events-none" />
      <ConnectorButton
        onAddAction={() => onInsertNode(afterNodeId, null, 'agent')}
        onAddScript={() => onInsertNode(afterNodeId, null, 'script')}
        onAddCondition={() => onInsertNode(afterNodeId, null, 'condition')}
        onAddApproval={() => onInsertNode(afterNodeId, null, 'approval')}
        onAddLoop={
          // Loop-inside-branch is untested against fork joins, same gate as the rail.
          !insideBranch ? () => onInsertNode(afterNodeId, null, 'loop') : undefined
        }
        onAddConnectorAction={() => onInsertNode(afterNodeId, null, 'connectorAction')}
        onAddParallelBranch={
          !insideBranch ? () => onAddParallelBranch(afterNodeId, 'agent') : undefined
        }
      />
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
  const { onInsertNode, onAddParallelBranch } = useInteractions()
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
            // Hovered, the + and its menu must rise above the cards (selection elevates those to 1000).
            zIndex: hovered ? 1300 : 'auto'
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
          {insertable && hovered && (
            <div>
              <ConnectorButton
                onAddAction={() =>
                  onInsertNode(edgeData!.afterNodeId, edgeData!.beforeNodeId, 'agent')
                }
                onAddScript={() =>
                  onInsertNode(edgeData!.afterNodeId, edgeData!.beforeNodeId, 'script')
                }
                onAddCondition={() =>
                  onInsertNode(edgeData!.afterNodeId, edgeData!.beforeNodeId, 'condition')
                }
                onAddApproval={() =>
                  onInsertNode(edgeData!.afterNodeId, edgeData!.beforeNodeId, 'approval')
                }
                onAddLoop={
                  !edgeData!.insideBranch
                    ? () => onInsertNode(edgeData!.afterNodeId, edgeData!.beforeNodeId, 'loop')
                    : undefined
                }
                onAddConnectorAction={() =>
                  onInsertNode(edgeData!.afterNodeId, edgeData!.beforeNodeId, 'connectorAction')
                }
                onAddParallelBranch={
                  !edgeData!.insideBranch
                    ? () => onAddParallelBranch(edgeData!.afterNodeId, 'agent')
                    : undefined
                }
              />
            </div>
          )}
        </div>
      </EdgeLabelRenderer>
    </g>
  )
}

const NODE_TYPES = { step: StepNode, loop: LoopNode, addStep: AddStepNode }
const EDGE_TYPES = { step: StepEdge }

function WorkflowCanvasInner({
  nodes,
  edges,
  onNodeClick,
  onInsertNode,
  onAddParallelBranch,
  onConnectEdge,
  onPaletteInsert,
  onPositionsCommit,
  onDeleteNode,
  onTidyUp,
  selectedNodeId,
  nodeStatus
}: Props) {
  const { screenToFlowPosition, zoomIn, zoomOut, zoomTo, fitView } = useReactFlow()
  const wrapperRef = useRef<HTMLDivElement>(null)

  const elements = useMemo(() => toCanvasElements(nodes, edges), [nodes, edges])
  const [rfNodes, setRfNodes] = useState<Node[]>(elements.nodes)

  // Adjust-state-while-rendering: rebuild from the definition; drag positions stay local.
  const [syncedElements, setSyncedElements] = useState(elements)
  if (syncedElements !== elements) {
    setSyncedElements(elements)
    setRfNodes(elements.nodes)
  }

  const handleNodesChange = useCallback((changes: NodeChange[]) => {
    // The canvas owns position only; selection and structure stay the editor's.
    const positional = changes.filter((c) => c.type === 'position' || c.type === 'dimensions')
    if (positional.length > 0) setRfNodes((prev) => applyNodeChanges(positional, prev))
  }, [])

  const handleDragStop = useCallback(() => {
    // Committing every displayed position materializes the computed layout on first drag.
    const positions: Record<string, { x: number; y: number }> = {}
    for (const rfNode of rfNodes) {
      if (rfNode.type === 'addStep') continue
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

  // --- Node search palette -------------------------------------------------
  const [palette, setPalette] = useState<{
    screen: { x: number; y: number }
    flow: { x: number; y: number }
    afterNodeId: string
  } | null>(null)
  const [connectorItems, setConnectorItems] = useState<PaletteConnectorItem[]>([])
  const connections = useConnections()

  useEffect(() => {
    if (!palette) return
    let cancelled = false
    Promise.all(
      connections.map(async (conn) => {
        try {
          const actions = await window.api.listConnectionActions(conn.id)
          return actions.map((a) => ({
            connectionId: conn.id,
            action: a.type,
            label: a.label || a.type,
            source: conn.name
          }))
        } catch {
          return []
        }
      })
    ).then((lists) => {
      if (!cancelled) setConnectorItems(lists.flat())
    })
    return () => {
      cancelled = true
    }
    // Loaded once per palette opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [palette !== null])

  const openPaletteAt = useCallback(
    (clientX: number, clientY: number, afterNodeId: string) => {
      const rect = wrapperRef.current?.getBoundingClientRect()
      if (!rect) return
      setPalette({
        screen: { x: clientX - rect.left, y: clientY - rect.top },
        flow: screenToFlowPosition({ x: clientX, y: clientY }),
        afterNodeId
      })
    },
    [screenToFlowPosition]
  )

  const pendingConnectSource = useRef<string | null>(null)

  const handleConnectStart = useCallback((_: unknown, params: { nodeId: string | null }) => {
    pendingConnectSource.current = params.nodeId
  }, [])

  const handleConnectEnd = useCallback(
    (event: MouseEvent | TouchEvent, connectionState: { isValid: boolean | null }) => {
      const source = pendingConnectSource.current
      pendingConnectSource.current = null
      // A drop on empty canvas opens the node search where the edge was released.
      if (connectionState.isValid === null && source && 'clientX' in event) {
        openPaletteAt(event.clientX, event.clientY, source)
      }
    },
    [openPaletteAt]
  )

  const leafForTabInsert = useCallback((): string | null => {
    const hasOutgoing = new Set(edges.map((e) => e.source))
    const leaf = [...nodes].reverse().find((n) => !hasOutgoing.has(n.id))
    return leaf?.id ?? null
  }, [nodes, edges])

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (palette) return
      const target = e.target as HTMLElement
      if (target.closest('input, textarea, [contenteditable]')) return
      if (e.key === 'Tab') {
        const afterNodeId = leafForTabInsert()
        const rect = wrapperRef.current?.getBoundingClientRect()
        if (!afterNodeId || !rect) return
        e.preventDefault()
        openPaletteAt(rect.left + rect.width / 2 - 124, rect.top + rect.height / 3, afterNodeId)
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
        if (node && node.type !== 'trigger' && onDeleteNode) {
          e.preventDefault()
          onDeleteNode(selectedNodeId)
        }
      }
    },
    [
      palette,
      leafForTabInsert,
      openPaletteAt,
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
      onInsertNode,
      onAddParallelBranch
    }),
    [nodes, selectedNodeId, nodeStatus, onNodeClick, onInsertNode, onAddParallelBranch]
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
          onPaneClick={() => {
            setPalette(null)
            onNodeClick('')
          }}
          fitView
          fitViewOptions={{ padding: 0.2, maxZoom: 1 }}
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

        {palette && (
          <NodePalette
            position={palette.screen}
            allowLoop={!elements.branchMembers.has(palette.afterNodeId)}
            connectorItems={connectorItems}
            onPick={(pick) => {
              onPaletteInsert(pick, palette.afterNodeId, palette.flow)
              setPalette(null)
            }}
            onClose={() => setPalette(null)}
          />
        )}
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

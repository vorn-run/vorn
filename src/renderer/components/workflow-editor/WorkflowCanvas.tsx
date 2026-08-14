import { useMemo, Fragment } from 'react'
import { TriggerNode } from './nodes/TriggerNode'
import { LaunchAgentNode } from './nodes/LaunchAgentNode'
import { ScriptNode } from './nodes/ScriptNode'
import { ConditionNode } from './nodes/ConditionNode'
import { ApprovalNode } from './nodes/ApprovalNode'
import { Repeat } from 'lucide-react'
// Fallback for a loop node that is not reachable from the trigger: orphans are
// appended as plain node rows, so they never reach LoopRenderer.
import { LoopNode } from './nodes/LoopNode'
import { CreateTaskFromItemNode } from './nodes/CreateTaskFromItemNode'
import { CallConnectorActionNode } from './nodes/CallConnectorActionNode'
import { ConnectorButton } from './nodes/AddStepNode'
import {
  WorkflowNode,
  WorkflowEdge,
  TriggerConfig,
  LaunchAgentConfig,
  ScriptConfig,
  ConditionConfig,
  ApprovalConfig,
  LoopConfig,
  CreateTaskFromItemConfig,
  CallConnectorActionConfig,
  NodeExecutionStatus
} from '../../../shared/types'
import { computeFlowLayout, FlowRow } from '../../lib/workflow-helpers'
import { NODE_SELECTED, NODE_UNSELECTED, NODE_GLYPH } from './node-visuals'
import { WORKFLOW_STATUS_DOT_PULSE } from '../../lib/workflow-status'

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
  selectedNodeId: string | null
  /**
   * What each node is doing in the runs that are live right now, so the canvas
   * reports a run rather than only describing a definition. Absent when nothing
   * is running, which is most of the time.
   */
  nodeStatus?: Record<string, NodeExecutionStatus>
}

function VerticalLine({ dashed, height }: { dashed?: boolean; height?: number }) {
  return (
    <div
      className={`w-px shrink-0 ${
        dashed ? 'border-l border-dashed border-white/[0.08]' : 'bg-white/[0.08]'
      }`}
      style={{ height: height ?? 24 }}
    />
  )
}

function NodeCard({
  node,
  allNodes,
  selected,
  onClick,
  executionStatus
}: {
  node: WorkflowNode
  /** Every node in the workflow: a loop card lists the steps it repeats. */
  allNodes: WorkflowNode[]
  selected: boolean
  onClick: () => void
  executionStatus?: NodeExecutionStatus
}) {
  if (node.type === 'loop') {
    return (
      <LoopNode
        label={node.label}
        config={node.config as LoopConfig}
        nodes={allNodes}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  if (node.type === 'trigger') {
    return (
      <TriggerNode
        label={node.label}
        config={node.config as TriggerConfig}
        selected={selected}
        onClick={onClick}
      />
    )
  }

  if (node.type === 'script') {
    return (
      <ScriptNode
        label={node.label}
        config={node.config as ScriptConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  if (node.type === 'condition') {
    return (
      <ConditionNode
        label={node.label}
        config={node.config as ConditionConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  if (node.type === 'approval') {
    return (
      <ApprovalNode
        label={node.label}
        config={node.config as ApprovalConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  if (node.type === 'createTaskFromItem') {
    return (
      <CreateTaskFromItemNode
        label={node.label}
        config={node.config as CreateTaskFromItemConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  if (node.type === 'callConnectorAction') {
    return (
      <CallConnectorActionNode
        label={node.label}
        config={node.config as CallConnectorActionConfig}
        selected={selected}
        onClick={onClick}
        executionStatus={executionStatus}
      />
    )
  }

  return (
    <LaunchAgentNode
      label={node.label}
      config={node.config as LaunchAgentConfig}
      selected={selected}
      onClick={onClick}
      executionStatus={executionStatus}
    />
  )
}

function FlowRowRenderer({
  rows,
  edges,
  nodes,
  onNodeClick,
  onInsertNode,
  onAddParallelBranch,
  selectedNodeId,
  nodeStatus,
  isInsideBranch
}: {
  rows: FlowRow[]
  edges?: WorkflowEdge[]
  nodes?: WorkflowNode[]
  onNodeClick: (nodeId: string) => void
  onInsertNode: (afterNodeId: string, beforeNodeId: string | null, type: AddableNodeType) => void
  onAddParallelBranch: (forkFromId: string, type: 'agent' | 'script') => void
  selectedNodeId: string | null
  isInsideBranch?: boolean
  nodeStatus?: Record<string, NodeExecutionStatus>
}) {
  return (
    <>
      {rows.map((row, i) => {
        if (row.kind === 'node') {
          const nextRow = rows[i + 1]
          const isLast = i === rows.length - 1
          const nextIsFork = nextRow?.kind === 'fork'

          let beforeNodeId: string | null = null
          if (nextIsFork) {
            beforeNodeId = '__FORK__'
          } else if (nextRow?.kind === 'node') {
            beforeNodeId = nextRow.node.id
          }

          return (
            <Fragment key={row.node.id}>
              {i > 0 && <VerticalLine />}

              <NodeCard
                node={row.node}
                allNodes={nodes ?? []}
                selected={row.node.id === selectedNodeId}
                onClick={() => onNodeClick(row.node.id)}
                executionStatus={nodeStatus?.[row.node.id]}
              />

              {!isLast && (
                <>
                  <VerticalLine />
                  <ConnectorButton
                    onAddAction={() => onInsertNode(row.node.id, beforeNodeId, 'agent')}
                    onAddScript={() => onInsertNode(row.node.id, beforeNodeId, 'script')}
                    onAddCondition={() => onInsertNode(row.node.id, beforeNodeId, 'condition')}
                    onAddApproval={() => onInsertNode(row.node.id, beforeNodeId, 'approval')}
                    onAddLoop={
                      // Gated like onAddParallelBranch: a loop lifts its body
                      // out of the trunk, and how that interacts with a fork's
                      // join is untested. Offering it inside a branch would be
                      // promising something that has never been tried.
                      !isInsideBranch
                        ? () => onInsertNode(row.node.id, beforeNodeId, 'loop')
                        : undefined
                    }
                    onAddConnectorAction={() =>
                      onInsertNode(row.node.id, beforeNodeId, 'connectorAction')
                    }
                    onAddParallelBranch={() => onAddParallelBranch(row.node.id, 'agent')}
                  />
                </>
              )}

              {isLast && (
                <>
                  <VerticalLine dashed />
                  <ConnectorButton
                    onAddAction={() => onInsertNode(row.node.id, null, 'agent')}
                    onAddScript={() => onInsertNode(row.node.id, null, 'script')}
                    onAddCondition={() => onInsertNode(row.node.id, null, 'condition')}
                    onAddApproval={() => onInsertNode(row.node.id, null, 'approval')}
                    onAddLoop={
                      !isInsideBranch ? () => onInsertNode(row.node.id, null, 'loop') : undefined
                    }
                    onAddConnectorAction={() => onInsertNode(row.node.id, null, 'connectorAction')}
                    onAddParallelBranch={
                      !isInsideBranch ? () => onAddParallelBranch(row.node.id, 'agent') : undefined
                    }
                  />
                </>
              )}
            </Fragment>
          )
        }

        if (row.kind === 'loop') {
          return (
            <LoopRenderer
              key={`loop-${row.loopNode.id}`}
              row={row}
              onNodeClick={onNodeClick}
              onInsertNode={onInsertNode}
              selectedNodeId={selectedNodeId}
              nodeStatus={nodeStatus}
              nodes={nodes ?? []}
            />
          )
        }

        return (
          <ForkRenderer
            key={`fork-${row.forkNodeId}`}
            row={row}
            onNodeClick={onNodeClick}
            onInsertNode={onInsertNode}
            onAddParallelBranch={onAddParallelBranch}
            selectedNodeId={selectedNodeId}
            nodeStatus={nodeStatus}
            edges={edges}
            nodes={nodes}
          />
        )
      })}
    </>
  )
}

function HorizontalBar({ branchCount }: { branchCount: number }) {
  return (
    <div className="flex w-full">
      {Array.from({ length: branchCount }, (_, i) => (
        <div key={i} className="flex-1 relative h-px">
          {i > 0 && <div className="absolute left-0 right-1/2 top-0 h-px bg-white/[0.08]" />}
          {i < branchCount - 1 && (
            <div className="absolute left-1/2 right-0 top-0 h-px bg-white/[0.08]" />
          )}
        </div>
      ))}
    </div>
  )
}

/**
 * A loop and the steps it repeats, drawn as one enclosure.
 *
 * The two questions a reader has — what repeats, and when does it stop — are
 * answered at the top and bottom edges of the rail, so neither requires
 * opening a panel. The body is inset, which is the whole point: a repeated
 * step must not look like a step that runs once.
 */
function LoopRenderer({
  row,
  onNodeClick,
  onInsertNode,
  selectedNodeId,
  nodeStatus,
  nodes
}: {
  row: Extract<FlowRow, { kind: 'loop' }>
  onNodeClick: (nodeId: string) => void
  onInsertNode: (afterNodeId: string, beforeNodeId: string | null, type: AddableNodeType) => void
  selectedNodeId: string | null
  nodeStatus?: Record<string, NodeExecutionStatus>
  nodes: WorkflowNode[]
}) {
  const config = row.loopNode.config as LoopConfig
  const selected = row.loopNode.id === selectedNodeId
  const until = config.until?.variable
    ? `until ${config.until.variable} ${config.until.operator} ${config.until.value}`
    : 'runs every pass'
  const lastBodyId = row.body.length > 0 ? row.body[row.body.length - 1] : null
  // The rail draws its own header instead of going through NodeCard, so the
  // loop is the one node whose status has to be read here. It has one: a loop
  // runs while it iterates, and errors when it has no body steps or holds a
  // gate — without this the rail sat plain while the run was inside it.
  const loopStatus = nodeStatus?.[row.loopNode.id]

  return (
    <div
      data-loop-rail
      className={`w-[312px] rounded-lg border transition-all
                  ${selected ? NODE_SELECTED : NODE_UNSELECTED}
                  bg-surface-node`}
    >
      <div
        onClick={(e) => {
          e.stopPropagation()
          onNodeClick(row.loopNode.id)
        }}
        className="flex items-center gap-2 px-4 py-2.5 border-b border-white/[0.06] cursor-pointer
                   hover:bg-white/[0.02] rounded-t-lg"
      >
        <Repeat size={13} className={`shrink-0 ${NODE_GLYPH}`} strokeWidth={2} />
        <span className="text-[12.5px] font-semibold text-white truncate flex-1">
          {row.loopNode.label}
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
        {row.body.length === 0 ? (
          <div
            className="w-full rounded-md border border-dashed border-white/[0.12] px-3 py-4
                       text-[11px] text-gray-500 text-center"
          >
            No steps yet — add one below
          </div>
        ) : (
          row.body.map((bodyRow, i) => (
            <Fragment key={bodyRow.kind === 'node' ? bodyRow.node.id : i}>
              {i > 0 && <VerticalLine />}
              {bodyRow.kind === 'node' && (
                <NodeCard
                  node={bodyRow.node}
                  allNodes={nodes}
                  selected={bodyRow.node.id === selectedNodeId}
                  onClick={() => onNodeClick(bodyRow.node.id)}
                  executionStatus={nodeStatus?.[bodyRow.node.id]}
                />
              )}
            </Fragment>
          ))
        )}

        {/* Inside the rail, so position is what decides membership. */}
        <VerticalLine />
        <ConnectorButton
          onAddAction={() =>
            onInsertNode(
              lastBodyId?.kind === 'node' ? lastBodyId.node.id : row.loopNode.id,
              '__LOOP_BODY__',
              'agent'
            )
          }
          onAddScript={() =>
            onInsertNode(
              lastBodyId?.kind === 'node' ? lastBodyId.node.id : row.loopNode.id,
              '__LOOP_BODY__',
              'script'
            )
          }
        />
      </div>

      <div className="px-4 pt-2.5 pb-3 text-[10px] font-mono text-gray-500 text-center truncate">
        ↻ {until}
      </div>
    </div>
  )
}

function ForkRenderer({
  row,
  onNodeClick,
  onInsertNode,
  onAddParallelBranch,
  selectedNodeId,
  nodeStatus,
  edges,
  nodes
}: {
  row: Extract<FlowRow, { kind: 'fork' }>
  onNodeClick: (nodeId: string) => void
  onInsertNode: (afterNodeId: string, beforeNodeId: string | null, type: AddableNodeType) => void
  onAddParallelBranch: (forkFromId: string, type: 'agent' | 'script') => void
  selectedNodeId: string | null
  nodeStatus?: Record<string, NodeExecutionStatus>
  edges?: WorkflowEdge[]
  nodes?: WorkflowNode[]
}) {
  const branchCount = row.branches.length

  // Check if this fork is from a condition node
  const forkNode = nodes?.find((n) => n.id === row.forkNodeId)
  const isConditionFork = forkNode?.type === 'condition'

  // Determine branch labels for condition forks
  const getBranchLabel = (branchIndex: number): string | null => {
    if (!isConditionFork || !edges) return null
    const branch = row.branches[branchIndex]
    const firstNodeInBranch = branch?.[0]?.kind === 'node' ? branch[0].node.id : null
    if (!firstNodeInBranch) return null
    const edge = edges.find(
      (e) => e.source === row.forkNodeId && e.target === firstNodeInBranch && e.conditionBranch
    )
    return edge?.conditionBranch === 'true'
      ? 'True'
      : edge?.conditionBranch === 'false'
        ? 'False'
        : null
  }

  return (
    <div className="flex flex-col items-center w-full">
      <HorizontalBar branchCount={branchCount} />

      <div className="flex w-full">
        {row.branches.map((branch, bi) => {
          const branchKey = branch[0]?.kind === 'node' ? branch[0].node.id : `branch-${bi}`
          const label = getBranchLabel(bi)

          return (
            <div
              key={branchKey}
              className="flex flex-col items-center flex-1"
              style={{ minWidth: 310 }}
            >
              {/* Both branches read the same. True was green and False red,
                  which said one path is the good one and the other a failure —
                  a condition is a fork, and the word already says which. */}
              {label && (
                <div className="px-2.5 py-0.5 rounded-full text-[10px] font-semibold mb-1 border border-white/[0.08] text-ink-secondary">
                  {label}
                </div>
              )}

              <VerticalLine />

              <FlowRowRenderer
                rows={branch}
                edges={edges}
                nodes={nodes}
                onNodeClick={onNodeClick}
                onInsertNode={onInsertNode}
                onAddParallelBranch={onAddParallelBranch}
                selectedNodeId={selectedNodeId}
                nodeStatus={nodeStatus}
                isInsideBranch
              />

              {row.joinNodeId && <VerticalLine />}
            </div>
          )
        })}
      </div>

      {row.joinNodeId && <HorizontalBar branchCount={branchCount} />}
    </div>
  )
}

export function WorkflowCanvas({
  nodes,
  edges,
  onNodeClick,
  onInsertNode,
  onAddParallelBranch,
  selectedNodeId,
  nodeStatus
}: Props) {
  const flowLayout = useMemo(() => computeFlowLayout(nodes, edges), [nodes, edges])

  return (
    <div className="flex-1 h-full overflow-auto" onClick={() => onNodeClick('')}>
      <div className="flex flex-col items-center py-8 min-h-full px-6">
        <FlowRowRenderer
          rows={flowLayout}
          edges={edges}
          nodes={nodes}
          onNodeClick={onNodeClick}
          onInsertNode={onInsertNode}
          onAddParallelBranch={onAddParallelBranch}
          selectedNodeId={selectedNodeId}
          nodeStatus={nodeStatus}
        />
      </div>
    </div>
  )
}

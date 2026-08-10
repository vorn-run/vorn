import { useMemo, Fragment } from 'react'
import { TriggerNode } from './nodes/TriggerNode'
import { LaunchAgentNode } from './nodes/LaunchAgentNode'
import { ScriptNode } from './nodes/ScriptNode'
import { ConditionNode } from './nodes/ConditionNode'
import { ApprovalNode } from './nodes/ApprovalNode'
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
  CallConnectorActionConfig
} from '../../../shared/types'
import { computeFlowLayout, FlowRow } from '../../lib/workflow-helpers'

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
  onClick
}: {
  node: WorkflowNode
  /** Every node in the workflow: a loop card lists the steps it repeats. */
  allNodes: WorkflowNode[]
  selected: boolean
  onClick: () => void
}) {
  if (node.type === 'loop') {
    return (
      <LoopNode
        label={node.label}
        config={node.config as LoopConfig}
        nodes={allNodes}
        selected={selected}
        onClick={onClick}
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
      />
    )
  }

  return (
    <LaunchAgentNode
      label={node.label}
      config={node.config as LaunchAgentConfig}
      selected={selected}
      onClick={onClick}
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
              />

              {!isLast && (
                <>
                  <VerticalLine />
                  <ConnectorButton
                    onAddAction={() => onInsertNode(row.node.id, beforeNodeId, 'agent')}
                    onAddScript={() => onInsertNode(row.node.id, beforeNodeId, 'script')}
                    onAddCondition={() => onInsertNode(row.node.id, beforeNodeId, 'condition')}
                    onAddApproval={() => onInsertNode(row.node.id, beforeNodeId, 'approval')}
                    onAddLoop={() => onInsertNode(row.node.id, beforeNodeId, 'loop')}
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
                    onAddLoop={() => onInsertNode(row.node.id, null, 'loop')}
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

        return (
          <ForkRenderer
            key={`fork-${row.forkNodeId}`}
            row={row}
            onNodeClick={onNodeClick}
            onInsertNode={onInsertNode}
            onAddParallelBranch={onAddParallelBranch}
            selectedNodeId={selectedNodeId}
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

function ForkRenderer({
  row,
  onNodeClick,
  onInsertNode,
  onAddParallelBranch,
  selectedNodeId,
  edges,
  nodes
}: {
  row: Extract<FlowRow, { kind: 'fork' }>
  onNodeClick: (nodeId: string) => void
  onInsertNode: (afterNodeId: string, beforeNodeId: string | null, type: AddableNodeType) => void
  onAddParallelBranch: (forkFromId: string, type: 'agent' | 'script') => void
  selectedNodeId: string | null
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
              {label && (
                <div
                  className={`px-2.5 py-0.5 rounded-full text-[10px] font-semibold mb-1 ${
                    label === 'True'
                      ? 'bg-green-500/20 text-green-400'
                      : 'bg-red-500/20 text-red-400'
                  }`}
                >
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
  selectedNodeId
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
        />
      </div>
    </div>
  )
}

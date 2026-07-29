import { useState, useMemo } from 'react'
import { ChevronRight } from 'lucide-react'
import { useAppStore } from '../../stores'
import { formatRelativeTime, formatCompactDuration } from '../../lib/format-time'
import { StatusDot, NodeLabel, RunStepsList } from '../workflow-editor/RunEntry'
import { LogReplayModal } from '../LogReplayModal'
import { StopRunButton } from './StopRunButton'
import {
  workflowRunId,
  type NodeExecutionState,
  type WorkflowExecution,
  type WorkflowNode
} from '../../../shared/types'
import type { RunListEntry } from '../../hooks/useAllWorkflowRuns'
import type { RunBucket } from '../../stores/types'

type Bucket = RunBucket

const NODE_PILL_CLASS: Partial<Record<NodeExecutionState['status'], string>> = {
  waiting: 'text-amber-400 border-amber-500/30',
  error: 'text-red-400 border-red-500/30'
}
const NODE_PILL_DEFAULT = 'text-gray-300 border-white/[0.08]'

const GRID_COLS = '14px minmax(180px,1.4fr) 110px 90px minmax(140px,1.1fr) 110px 16px'

function bucketOf(execution: WorkflowExecution): Bucket {
  if (execution.status === 'running') {
    return execution.nodeStates.some((n) => n.status === 'waiting') ? 'waiting' : 'running'
  }
  return execution.status === 'success' ? 'success' : 'error'
}

function lastNodeOf(execution: WorkflowExecution): NodeExecutionState | undefined {
  const waiting = execution.nodeStates.find((n) => n.status === 'waiting')
  if (waiting) return waiting
  const sortable = execution.nodeStates.filter(
    (n): n is NodeExecutionState & { startedAt: string } => typeof n.startedAt === 'string'
  )
  sortable.sort((a, b) => (a.startedAt > b.startedAt ? -1 : 1))
  return sortable[0]
}

interface Row {
  run: RunListEntry
  bucket: Bucket
  last: NodeExecutionState | undefined
}

interface Props {
  runs: RunListEntry[]
  workflowsById: Map<string, { name?: string; nodes: WorkflowNode[] }>
  filter: Bucket
}

export function AllRunsTable({ runs, workflowsById, filter }: Props) {
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [logModal, setLogModal] = useState<string | null>(null)
  const setMainViewMode = useAppStore((s) => s.setMainViewMode)
  const setEditingWorkflowId = useAppStore((s) => s.setEditingWorkflowId)

  const rows: Row[] = useMemo(
    () => runs.map((run) => ({ run, bucket: bucketOf(run), last: lastNodeOf(run) })),
    [runs]
  )

  const visible = useMemo(
    () => (filter === 'all' ? rows : rows.filter((r) => r.bucket === filter)),
    [filter, rows]
  )

  const openInEditor = (workflowId: string): void => {
    setEditingWorkflowId(workflowId)
    setMainViewMode('workflows')
  }

  return (
    <div className="flex flex-col">
      <div className="text-[12px]">
        <div
          className="grid items-center px-3 py-2 border-b border-white/[0.04]
                     text-[10px] uppercase tracking-wider text-gray-600"
          style={{ gridTemplateColumns: GRID_COLS }}
        >
          <span></span>
          <span>Workflow</span>
          <span>Started</span>
          <span>Duration</span>
          <span>Trigger</span>
          <span>Last node</span>
          <span></span>
        </div>

        {visible.length === 0 ? (
          <div className="text-center py-10 text-gray-600 text-[12px]">No runs to show</div>
        ) : (
          visible.map(({ run, bucket, last }) => {
            const id = workflowRunId(run)
            const expanded = expandedId === id
            const wf = workflowsById.get(run.workflowId)
            const wfNodes = wf?.nodes ?? []
            const liveName = wf?.name?.trim() || run.workflowName?.trim()
            const isDeleted = !wf
            return (
              <div key={id}>
                <button
                  type="button"
                  onClick={() => setExpandedId(expanded ? null : id)}
                  onDoubleClick={isDeleted ? undefined : () => openInEditor(run.workflowId)}
                  className={`w-full grid items-center px-3 py-2.5 text-left
                              border-b border-white/[0.04]
                              ${expanded ? 'bg-white/[0.04] text-white' : 'text-gray-300 hover:bg-white/[0.04] hover:text-white'}`}
                  style={{ gridTemplateColumns: GRID_COLS }}
                >
                  <StatusDot status={bucket === 'waiting' ? 'waiting' : run.status} />
                  <span
                    className="min-w-0 truncate flex items-center gap-1.5"
                    title={
                      liveName
                        ? `${liveName} · ${run.workflowId}`
                        : `${run.workflowId} (workflow deleted)`
                    }
                  >
                    {liveName ? (
                      <span className="text-white truncate">{liveName}</span>
                    ) : (
                      <span className="font-mono text-[12px] text-gray-500 truncate">
                        {run.workflowId.slice(0, 8)}
                      </span>
                    )}
                    {isDeleted && (
                      <span className="text-[10px] uppercase tracking-wide text-gray-600 shrink-0">
                        deleted
                      </span>
                    )}
                  </span>
                  <span className="font-mono tabular-nums text-gray-500">
                    {formatRelativeTime(run.startedAt)}
                  </span>
                  <span className="font-mono tabular-nums text-gray-500">
                    {formatCompactDuration(run.startedAt, run.completedAt)}
                  </span>
                  <span className="min-w-0 truncate">
                    {run.triggerTaskId ? (
                      <span
                        className="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-mono
                                   text-violet-400 bg-violet-500/10 border border-violet-500/20"
                      >
                        task · {run.triggerTaskId.slice(0, 6)}
                      </span>
                    ) : (
                      <span className="text-gray-500 text-[11px]">manual</span>
                    )}
                  </span>
                  <span className="min-w-0 truncate font-mono text-[11px]">
                    {last ? (
                      <span
                        className={`inline-block px-1.5 py-0.5 rounded border ${
                          NODE_PILL_CLASS[last.status] ?? NODE_PILL_DEFAULT
                        }`}
                      >
                        <NodeLabel nodeId={last.nodeId} nodes={wfNodes} />
                      </span>
                    ) : (
                      <span className="text-gray-600">—</span>
                    )}
                  </span>
                  <ChevronRight
                    size={12}
                    className={`text-gray-600 transition-transform ${expanded ? 'rotate-90 text-white' : ''}`}
                  />
                </button>

                {expanded && (
                  <div className="bg-[#141416] border-b border-white/[0.04]">
                    <RunStepsList execution={run} nodes={wfNodes} onViewFullOutput={setLogModal} />
                    <div className="px-4 py-2 flex items-center justify-end gap-2">
                      <StopRunButton execution={run} />
                      <button
                        type="button"
                        disabled={isDeleted}
                        onClick={() => openInEditor(run.workflowId)}
                        title={isDeleted ? 'Workflow no longer exists' : undefined}
                        className="px-2 py-1 text-[11px] text-gray-400 border border-white/[0.08] rounded hover:bg-white/[0.04] hover:text-white disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent disabled:hover:text-gray-400"
                      >
                        Open workflow
                      </button>
                    </div>
                  </div>
                )}
              </div>
            )
          })
        )}
      </div>

      {logModal !== null && <LogReplayModal logs={logModal} onClose={() => setLogModal(null)} />}
    </div>
  )
}

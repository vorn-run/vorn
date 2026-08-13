import { useMemo } from 'react'
import { useAppStore } from '../../stores'
import { RunListRow } from './RunListRow'
import { bucketOf, type RunWorkflowRef } from '../../lib/run-presentation'
import { workflowRunId } from '../../../shared/types'
import type { RunListEntry } from '../../hooks/useAllWorkflowRuns'
import type { RunBucket } from '../../stores/types'

interface Props {
  runs: RunListEntry[]
  workflowsById: Map<string, RunWorkflowRef>
  filter: RunBucket
  selectedId: string | null
  onSelect: (id: string) => void
}

export function RunsList({ runs, workflowsById, filter, selectedId, onSelect }: Props) {
  const setMainViewMode = useAppStore((s) => s.setMainViewMode)
  const setEditingWorkflowId = useAppStore((s) => s.setEditingWorkflowId)

  const visible = useMemo(
    () => (filter === 'all' ? runs : runs.filter((r) => bucketOf(r) === filter)),
    [filter, runs]
  )

  const waitingCount = useMemo(
    () => visible.filter((r) => r.nodeStates.some((n) => n.status === 'waiting')).length,
    [visible]
  )

  const openInEditor = (workflowId: string): void => {
    setEditingWorkflowId(workflowId)
    setMainViewMode('workflows')
  }

  return (
    <div className="flex flex-col min-h-0 h-full">
      <div className="flex items-center gap-2 px-4 py-3 shrink-0">
        <h2 className="text-[15px] text-white">{filter === 'all' ? 'All runs' : 'Runs'}</h2>
        <span className="font-mono text-[11px] text-gray-500 tabular-nums px-1.5 py-0.5 rounded bg-white/[0.05]">
          {visible.length}
        </span>
        {waitingCount > 0 && (
          <span className="font-mono text-[11px] text-bronzo tabular-nums px-1.5 py-0.5 rounded bg-bronzo/10 border border-bronzo/20">
            {waitingCount} waiting
          </span>
        )}
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto border-t border-white/[0.04]">
        {visible.length === 0 ? (
          <p className="text-center py-10 text-gray-600 text-[12px]">No runs to show</p>
        ) : (
          visible.map((run) => {
            const id = workflowRunId(run)
            const wf = workflowsById.get(run.workflowId)
            return (
              <RunListRow
                key={id}
                run={run}
                // A deleted workflow leaves only the name persisted on the run.
                workflow={wf ?? { name: run.workflowName?.trim() || undefined, nodes: [] }}
                workflowDeleted={!wf}
                selected={selectedId === id}
                onSelect={() => onSelect(id)}
                onOpenWorkflow={() => openInEditor(run.workflowId)}
              />
            )
          })
        )}
      </div>
    </div>
  )
}

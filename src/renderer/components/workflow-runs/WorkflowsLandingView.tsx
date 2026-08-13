import { useEffect, useMemo, useState } from 'react'
import { useAllWorkflowRuns } from '../../hooks/useAllWorkflowRuns'
import { useAppStore } from '../../stores'
import { RunsList } from './RunsList'
import { bucketOf, type RunWorkflowRef } from '../../lib/run-presentation'
import { RunDetailPane, RunDetailEmptyState } from './RunDetailPane'
import { LogReplayModal } from '../LogReplayModal'
import { useRunsListResize } from './useRunsListResize'
import { workflowRunId } from '../../../shared/types'
import type { RunBucket } from '../../stores/types'

export function WorkflowsLandingView() {
  const tab = useAppStore((s) => s.workflowsLandingTab)
  const storeFilter = useAppStore((s) => s.workflowsRunFilter)
  const selectedRunId = useAppStore((s) => s.selectedRunId)
  const setSelectedRunId = useAppStore((s) => s.setSelectedRunId)
  const setMainViewMode = useAppStore((s) => s.setMainViewMode)
  const setEditingWorkflowId = useAppStore((s) => s.setEditingWorkflowId)
  const tasks = useAppStore((s) => s.config?.tasks)
  const workflows = useAppStore((s) => s.config?.workflows)
  const { runs } = useAllWorkflowRuns(50)
  const [logModal, setLogModal] = useState<string | null>(null)
  const { listWidth, isResizing, handleResizeStart, resetWidth } = useRunsListResize()

  const workflowsById = useMemo(
    () =>
      new Map<string, RunWorkflowRef>(
        (workflows ?? []).map((w) => [
          w.id,
          { name: w.name, icon: w.icon, iconColor: w.iconColor, nodes: w.nodes ?? [] }
        ])
      ),
    [workflows]
  )

  // The Needs review tab is the same surface narrowed to paused runs, so a
  // reviewer keeps the trace and the approve/reject actions they already know.
  const filter: RunBucket = tab === 'review' ? 'waiting' : storeFilter

  const visible = useMemo(
    () => (filter === 'all' ? runs : runs.filter((r) => bucketOf(r) === filter)),
    [filter, runs]
  )

  const selected = useMemo(
    () => visible.find((r) => workflowRunId(r) === selectedRunId),
    [visible, selectedRunId]
  )

  // Keep a selection alive as the list changes underneath: fall back to the
  // newest visible run whenever the current pick is filtered away or finishes
  // and leaves the bucket.
  useEffect(() => {
    if (visible.length === 0) {
      if (selectedRunId !== null) setSelectedRunId(null)
      return
    }
    if (!visible.some((r) => workflowRunId(r) === selectedRunId)) {
      setSelectedRunId(workflowRunId(visible[0]))
    }
  }, [visible, selectedRunId, setSelectedRunId])

  const selectedWorkflow = selected ? workflowsById.get(selected.workflowId) : undefined

  const openInEditor = (workflowId: string): void => {
    setEditingWorkflowId(workflowId)
    setMainViewMode('workflows')
  }

  return (
    <div className="flex-1 flex min-h-0 overflow-hidden">
      <div className="shrink-0 min-w-0" style={{ width: listWidth }}>
        <RunsList
          runs={runs}
          workflowsById={workflowsById}
          filter={filter}
          selectedId={selectedRunId}
          onSelect={setSelectedRunId}
        />
      </div>

      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize runs list"
        onPointerDown={handleResizeStart}
        onDoubleClick={resetWidth}
        className={`w-px shrink-0 cursor-col-resize relative transition-colors ${
          isResizing ? 'bg-white/40' : 'bg-white/[0.06] hover:bg-white/[0.16]'
        }`}
      >
        <span className="absolute inset-y-0 -left-1 -right-1" />
      </div>

      <div className="flex-1 min-w-0 min-h-0">
        {selected ? (
          <RunDetailPane
            key={workflowRunId(selected)}
            run={selected}
            workflow={
              selectedWorkflow ?? { name: selected.workflowName?.trim() || undefined, nodes: [] }
            }
            workflowDeleted={!selectedWorkflow}
            tasks={tasks}
            shortcutsEnabled={logModal === null}
            onOpenWorkflow={() => openInEditor(selected.workflowId)}
            onViewFullOutput={setLogModal}
          />
        ) : (
          <RunDetailEmptyState />
        )}
      </div>

      {logModal !== null && <LogReplayModal logs={logModal} onClose={() => setLogModal(null)} />}
    </div>
  )
}

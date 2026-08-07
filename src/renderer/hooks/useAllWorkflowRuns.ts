import { useEffect, useMemo, useState, useCallback, useRef } from 'react'
import { useAppStore } from '../stores'
import { rescheduleWaitingGateTimers } from '../lib/workflow-execution'
import { workflowRunId, type WorkflowExecution } from '../../shared/types'

export type RunListEntry = WorkflowExecution & { workflowName?: string }

/**
 * Loads recent workflow runs across the active workspace from the persistent
 * SQLite store. Live in-memory `workflowExecutions` override the snapshot for
 * matching workflows so an in-flight run shows fresh node states without a
 * reload, and `reload()` is debounced when an active run completes so the
 * snapshot doesn't go stale once it leaves the live Map.
 */
export function useAllWorkflowRuns(limit = 50): {
  runs: RunListEntry[]
  loading: boolean
  reload: () => Promise<void>
} {
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)
  const workflowExecutions = useAppStore((s) => s.workflowExecutions)
  const workflows = useAppStore((s) => s.config?.workflows)
  const reloadToken = useAppStore((s) => s.workflowsRunsReloadToken)
  const beginLoad = useAppStore((s) => s.beginWorkflowsRunsLoad)
  const endLoad = useAppStore((s) => s.endWorkflowsRunsLoad)
  const [persisted, setPersisted] = useState<RunListEntry[]>([])
  const [loading, setLoading] = useState(true)

  // Store-side loader uses an in-flight counter (not a boolean) so the
  // header spinner stays on while initial load, manual refresh, and the
  // debounced live-completion refetch overlap.
  const reload = useCallback(async () => {
    setLoading(true)
    beginLoad()
    try {
      const rows = await window.api.listAllWorkflowRuns(activeWorkspace, limit)
      setPersisted(rows)
      // A gate opened by another window is only in SQLite — without this the
      // run shows as `running` here and never offers its approve/reject
      // actions until the app restarts and App.tsx re-hydrates. Timers are
      // rescheduled for whatever this pass adopted, exactly as App.tsx does on
      // boot: whichever hydration wins the race, the gate timeout and the
      // connector lease heartbeat still get started.
      const waiting = await window.api.listRunsWithWaitingGates()
      const store = useAppStore.getState()
      const hydrated: WorkflowExecution[] = []
      for (const run of waiting) {
        if (store.workflowExecutions.has(run.runId)) continue
        store.setWorkflowExecution(run.runId, run)
        hydrated.push(run)
      }
      if (hydrated.length > 0) {
        rescheduleWaitingGateTimers(hydrated, store.config?.workflows ?? [])
      }
    } catch (err) {
      console.error('[useAllWorkflowRuns] load failed', err)
    } finally {
      setLoading(false)
      endLoad()
    }
  }, [activeWorkspace, limit, beginLoad, endLoad])

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: fetches the run snapshot on mount and whenever the workspace or reload token changes
    void reload()
  }, [reload, reloadToken])

  // When an in-memory run completes (status flips to success/error), the
  // persisted snapshot is stale — schedule a debounced refetch so the row's
  // final state survives a navigation away from the view.
  const liveActiveCount = useMemo(() => {
    let n = 0
    for (const exec of workflowExecutions.values()) if (exec.status === 'running') n++
    return n
  }, [workflowExecutions])
  const prevLiveActive = useRef(liveActiveCount)
  useEffect(() => {
    if (liveActiveCount < prevLiveActive.current) {
      const t = setTimeout(() => void reload(), 500)
      prevLiveActive.current = liveActiveCount
      return () => clearTimeout(t)
    }
    prevLiveActive.current = liveActiveCount
    return undefined
  }, [liveActiveCount, reload])

  const runs = useMemo<RunListEntry[]>(() => {
    const out: RunListEntry[] = []
    // Keyed by run: several runs of one workflow can be in flight at once, so
    // matching a snapshot row to "the" live run by workflow id would collapse
    // them onto each other.
    const live = new Map<string, WorkflowExecution>(workflowExecutions)

    for (const r of persisted) {
      const key = workflowRunId(r)
      const liveExec = live.get(key)
      if (liveExec) {
        // The live entry is normally the fresher one — except when it says the
        // run is still going and the database says it already finished. That
        // only happens for a run another window owns, whose live copy this
        // renderer hydrated and can never update. Trusting it there would show
        // a stale gate and let its Approve button re-run a finished workflow.
        const staleLive = liveExec.status === 'running' && r.status !== 'running'
        out.push(staleLive ? r : { ...liveExec, workflowName: r.workflowName })
        live.delete(key)
      } else {
        out.push(r)
      }
    }
    for (const exec of live.values()) {
      const wf = workflows?.find((w) => w.id === exec.workflowId)
      if (wf && (wf.workspaceId ?? 'personal') !== activeWorkspace) continue
      out.push({ ...exec, workflowName: wf?.name })
    }
    out.sort((a, b) => (a.startedAt < b.startedAt ? 1 : -1))
    return out
  }, [persisted, workflowExecutions, workflows, activeWorkspace])

  return { runs, loading, reload }
}

import { RefreshCw } from 'lucide-react'
import { useAppStore } from '../../stores'
import { useWaitingApprovals } from '../../hooks/useWaitingApprovals'
import { Tooltip } from '../Tooltip'
import { RunsToolbar } from './RunsToolbar'

export function WorkflowsLandingHeader() {
  const tab = useAppStore((s) => s.workflowsLandingTab)
  const setTab = useAppStore((s) => s.setWorkflowsLandingTab)
  const filter = useAppStore((s) => s.workflowsRunFilter)
  const setFilter = useAppStore((s) => s.setWorkflowsRunFilter)
  const loading = useAppStore((s) => s.workflowsRunsInflight > 0)
  const bumpReload = useAppStore((s) => s.bumpWorkflowsRunsReload)
  const waiting = useWaitingApprovals()

  return (
    <div className="flex items-center gap-1 titlebar-no-drag">
      <button
        type="button"
        onClick={() => setTab('runs')}
        className={`px-2.5 py-1 rounded-md text-[12px] transition-colors ${
          tab === 'runs'
            ? 'text-white bg-white/[0.08]'
            : 'text-gray-400 hover:text-white hover:bg-white/[0.06]'
        }`}
      >
        All runs
      </button>
      <button
        type="button"
        onClick={() => setTab('review')}
        className={`px-2.5 py-1 rounded-md text-[12px] transition-colors flex items-center gap-1.5 ${
          tab === 'review'
            ? 'text-white bg-white/[0.08]'
            : 'text-gray-400 hover:text-white hover:bg-white/[0.06]'
        }`}
      >
        Needs review
        {waiting.length > 0 && (
          <span className="font-mono text-[10px] text-bronzo tabular-nums">{waiting.length}</span>
        )}
      </button>

      <div className="w-px h-4 bg-white/[0.06] mx-1" />

      {tab === 'runs' && <RunsToolbar filter={filter} setFilter={setFilter} />}

      <Tooltip label="Refresh" position="bottom">
        <button
          type="button"
          onClick={bumpReload}
          aria-label="Refresh"
          className="p-1 rounded-md text-gray-400 hover:text-white hover:bg-white/[0.06] transition-colors"
        >
          <RefreshCw size={14} strokeWidth={1.5} className={loading ? 'animate-spin' : ''} />
        </button>
      </Tooltip>
    </div>
  )
}

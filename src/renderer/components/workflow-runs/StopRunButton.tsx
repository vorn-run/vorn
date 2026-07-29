import { useState } from 'react'
import { Square } from 'lucide-react'
import { Tooltip } from '../Tooltip'
import { isRunStoppable, stopWorkflowRun } from '../../lib/workflow-execution'
import type { WorkflowExecution } from '../../../shared/types'

interface Props {
  execution: WorkflowExecution
  /** Sits inside a clickable row; stops the click reaching it. */
  stopPropagation?: boolean
}

/**
 * Ends a run that is still going: kills the agents it launched and closes it as
 * stopped. Worktrees stay on disk, so whatever the agents managed to do is
 * still there to look at.
 *
 * Renders nothing once the run is finished — there is nothing left to stop.
 */
export function StopRunButton({ execution, stopPropagation = true }: Props) {
  const [stopping, setStopping] = useState(false)

  if (!isRunStoppable(execution)) return null

  const handleClick = async (e: React.MouseEvent): Promise<void> => {
    if (stopPropagation) e.stopPropagation()
    if (stopping) return
    setStopping(true)
    try {
      await stopWorkflowRun(execution.runId)
    } catch (err) {
      // An async click handler that throws becomes an unhandled rejection and
      // the run silently appears not to stop. Say so instead.
      console.error(`[workflow] failed to stop run ${execution.runId}`, err)
      // Imported here rather than at the top so a leaf button does not pull the
      // toast system into the module graph of every run row that renders it.
      const { toast } = await import('../Toast')
      toast.error('Could not stop the run')
    } finally {
      setStopping(false)
    }
  }

  return (
    <Tooltip label={stopping ? 'Stopping run' : 'Stop run'}>
      <button
        type="button"
        aria-label="Stop run"
        disabled={stopping}
        onClick={handleClick}
        className="p-1 rounded text-gray-500 hover:text-white hover:bg-white/[0.06]
                   transition-colors disabled:opacity-40 disabled:cursor-not-allowed
                   disabled:hover:bg-transparent disabled:hover:text-gray-500"
      >
        <Square size={11} strokeWidth={2.5} />
      </button>
    </Tooltip>
  )
}

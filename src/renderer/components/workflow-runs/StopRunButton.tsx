import { useState } from 'react'
import { Square } from 'lucide-react'
import { IconButton } from '../IconButton'
import { isRunStoppable } from '@vornrun/shared/workflow-graph'
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
      await window.api.stopWorkflowRun(execution.runId)
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
    <IconButton
      label="Stop run"
      hint={stopping ? 'Stopping run' : undefined}
      disabled={stopping}
      onClick={handleClick}
    >
      <Square size={11} strokeWidth={2.5} />
    </IconButton>
  )
}
